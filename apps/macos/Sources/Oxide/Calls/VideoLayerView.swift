import AVFoundation
import AppKit
import LiveKitWebRTC
import os

/// A layer-backed view fed by a WebRTC video track.
final class VideoLayerView: NSView {
    private let display = AVSampleBufferDisplayLayer()
    private let renderer: FrameRenderer
    private var track: LKRTCVideoTrack?

    /// `fit`: the whole picture, letterboxed, for a shared screen whose edges matter; cameras fill.
    init(mirrored: Bool, fit: Bool = false) {
        renderer = FrameRenderer(layer: display)
        super.init(frame: .zero)
        wantsLayer = true
        layer = CALayer()
        layer?.backgroundColor = NSColor.black.cgColor
        display.videoGravity = fit ? .resizeAspect : .resizeAspectFill
        if mirrored { display.setAffineTransform(CGAffineTransform(scaleX: -1, y: 1)) }
        layer?.addSublayer(display)
    }
    required init?(coder: NSCoder) { fatalError() }

    override func layout() {
        super.layout()
        display.frame = bounds
    }

    func attach(_ next: LKRTCVideoTrack?) {
        guard next !== track else { return }
        track?.remove(renderer)
        track = next
        next?.add(renderer)
    }
}

/// Receives frames on WebRTC threads and enqueues them for display.
private final class FrameRenderer: NSObject, LKRTCVideoRenderer, @unchecked Sendable {
    private let layer: AVSampleBufferDisplayLayer
    /// A frame is on its way to the main thread; newer ones are dropped until it is enqueued.
    private let pending = OSAllocatedUnfairLock(initialState: false)
    /// Reused NV12 buffers for software-decoded frames, rebuilt when the resolution changes.
    private let pool = OSAllocatedUnfairLock<(pool: CVPixelBufferPool, width: Int, height: Int)?>(uncheckedState: nil)

    init(layer: AVSampleBufferDisplayLayer) { self.layer = layer }

    func setSize(_ size: CGSize) {}

    func renderFrame(_ frame: LKRTCVideoFrame?) {
        guard let frame else { return }
        let busy = pending.withLock { busy in
            defer { busy = true }
            return busy
        }
        guard !busy else { return }
        guard let pixels = pixelBuffer(frame.buffer), let sample = Self.sample(pixels) else {
            pending.withLock { $0 = false }
            return
        }
        DispatchQueue.main.async { [layer, pending] in
            if layer.status == .failed { layer.flush() }
            layer.enqueue(sample)
            pending.withLock { $0 = false }
        }
    }

    private func makeBuffer(width: Int, height: Int) -> CVPixelBuffer? {
        pool.withLockUnchecked { current -> CVPixelBuffer? in
            if current?.width != width || current?.height != height {
                let attributes = [
                    kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                    kCVPixelBufferWidthKey: width,
                    kCVPixelBufferHeightKey: height,
                    kCVPixelBufferIOSurfacePropertiesKey: [:] as CFDictionary,
                ] as CFDictionary
                var created: CVPixelBufferPool?
                guard CVPixelBufferPoolCreate(nil, nil, attributes, &created) == kCVReturnSuccess, let created else {
                    current = nil
                    return nil
                }
                current = (created, width, height)
            }
            guard let pool = current?.pool else { return nil }
            var out: CVPixelBuffer?
            guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &out) == kCVReturnSuccess else { return nil }
            return out
        }
    }

    private func pixelBuffer(_ buffer: LKRTCVideoFrameBuffer) -> CVPixelBuffer? {
        if let native = buffer as? LKRTCCVPixelBuffer { return native.pixelBuffer }
        // Software-decoded frames: I420 → NV12.
        let i420 = buffer.toI420()
        let width = Int(i420.width), height = Int(i420.height)
        guard let out = makeBuffer(width: width, height: height) else { return nil }
        CVPixelBufferLockBaseAddress(out, [])
        defer { CVPixelBufferUnlockBaseAddress(out, []) }
        guard let y = CVPixelBufferGetBaseAddressOfPlane(out, 0)?.assumingMemoryBound(to: UInt8.self),
              let uv = CVPixelBufferGetBaseAddressOfPlane(out, 1)?.assumingMemoryBound(to: UInt8.self)
        else { return nil }
        let yStride = CVPixelBufferGetBytesPerRowOfPlane(out, 0)
        let uvStride = CVPixelBufferGetBytesPerRowOfPlane(out, 1)
        for row in 0..<height {
            (y + row * yStride).update(from: i420.dataY + row * Int(i420.strideY), count: width)
        }
        let chromaWidth = Int(i420.chromaWidth), chromaHeight = Int(i420.chromaHeight)
        for row in 0..<chromaHeight {
            let u = i420.dataU + row * Int(i420.strideU)
            let v = i420.dataV + row * Int(i420.strideV)
            let target = uv + row * uvStride
            for column in 0..<chromaWidth {
                target[2 * column] = u[column]
                target[2 * column + 1] = v[column]
            }
        }
        return out
    }

    private static func sample(_ pixels: CVPixelBuffer) -> CMSampleBuffer? {
        var format: CMVideoFormatDescription?
        guard CMVideoFormatDescriptionCreateForImageBuffer(allocator: nil, imageBuffer: pixels,
                                                           formatDescriptionOut: &format) == noErr,
              let format else { return nil }
        var timing = CMSampleTimingInfo(duration: .invalid, presentationTimeStamp: .zero, decodeTimeStamp: .invalid)
        var sample: CMSampleBuffer?
        guard CMSampleBufferCreateReadyWithImageBuffer(allocator: nil, imageBuffer: pixels, formatDescription: format,
                                                       sampleTiming: &timing, sampleBufferOut: &sample) == noErr,
              let sample else { return nil }
        if let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: true),
           CFArrayGetCount(attachments) > 0 {
            let dictionary = unsafeBitCast(CFArrayGetValueAtIndex(attachments, 0), to: CFMutableDictionary.self)
            CFDictionarySetValue(dictionary, Unmanaged.passUnretained(kCMSampleAttachmentKey_DisplayImmediately).toOpaque(),
                                 Unmanaged.passUnretained(kCFBooleanTrue).toOpaque())
        }
        return sample
    }
}
