import Darwin
import SwiftUI
import OxideCore

/// What a part of the address space is used for, from the tag the system puts on each VM region.
enum MemoryKind: CaseIterable {
    case heap, graphics, images, media, webKit, stacks, code, other

    var label: String {
        switch self {
        case .heap: L("ui.debug.kind_heap")
        case .graphics: L("ui.debug.kind_graphics")
        case .images: L("ui.debug.kind_images")
        case .media: L("ui.debug.kind_media")
        case .webKit: L("ui.debug.kind_webkit")
        case .stacks: L("ui.debug.kind_stacks")
        case .code: L("ui.debug.kind_code")
        case .other: L("ui.debug.kind_other")
        }
    }

    /// `VM_MEMORY_*` tags from <mach/vm_statistics.h>; untagged file mappings are code and data of libraries.
    init(tag: UInt32, fileBacked: Bool) {
        switch tag {
        case 1...9, 11...13: self = .heap
        case 21, 42, 51, 54...58, 66, 68, 88, 100: self = .graphics
        case 52, 70, 76, 77, 103: self = .images
        case 90...96, 101, 106: self = .media
        case 63...65, 69: self = .webKit
        case 30, 31: self = .stacks
        case 33, 60, 61, 82, 83: self = .code
        case 0 where fileBacked: self = .code
        default: self = .other
        }
    }
}

/// Where the app's memory goes at one moment, for Settings › Debug.
struct MemoryReport {
    /// What Activity Monitor shows as Memory.
    var footprint: UInt64 = 0
    var peak: UInt64 = 0
    var resident: UInt64 = 0
    /// Dirty and compressed bytes per kind, largest first.
    var kinds: [(kind: MemoryKind, bytes: UInt64)] = []
    var heapInUse: UInt64 = 0
    var heapAllocated: UInt64 = 0
    var rustLive: UInt64 = 0
    var rustPeak: UInt64 = 0
    var rustAllocations: UInt64 = 0
    var avatars = Store.ImageMemory()
    var previews = Store.ImageMemory()
    var cache = PictureCache(previewFiles: 0, previewBytes: 0, photoFiles: 0, photoBytes: 0)

    /// Reads the process counters and walks its regions; a few milliseconds, so off the main thread.
    nonisolated static func measure() -> MemoryReport {
        var report = MemoryReport()
        if let info = taskInfo() {
            report.footprint = info.phys_footprint
            report.peak = UInt64(info.ledger_phys_footprint_peak)
            report.resident = info.resident_size
        }
        var totals: [MemoryKind: UInt64] = [:]
        let page = UInt64(vm_kernel_page_size)
        var address: mach_vm_address_t = 0
        var size: mach_vm_size_t = 0
        var depth: natural_t = 0
        while true {
            var region = vm_region_submap_info_data_64_t()
            var count = mach_msg_type_number_t(
                MemoryLayout<vm_region_submap_info_data_64_t>.size / MemoryLayout<natural_t>.size)
            let status = withUnsafeMutablePointer(to: &region) {
                $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                    mach_vm_region_recurse(mach_task_self_, &address, &size, &depth, $0, &count)
                }
            }
            guard status == KERN_SUCCESS else { break }
            if region.is_submap != 0 {
                depth += 1
                continue
            }
            let bytes = (UInt64(region.pages_dirtied) + UInt64(region.pages_swapped_out)) * page
            if bytes > 0 {
                totals[MemoryKind(tag: region.user_tag, fileBacked: region.external_pager != 0), default: 0] += bytes
            }
            address += size
        }
        report.kinds = totals.map { ($0.key, $0.value) }.sorted { $0.bytes > $1.bytes }
        var heap = malloc_statistics_t()
        malloc_zone_statistics(nil, &heap)
        report.heapInUse = UInt64(heap.size_in_use)
        report.heapAllocated = UInt64(heap.size_allocated)
        let rust = rustMemory()
        report.rustLive = rust.liveBytes
        report.rustPeak = rust.peakBytes
        report.rustAllocations = rust.allocations
        report.cache = pictureCache()
        return report
    }

    /// The footprint alone, cheap enough for the settings home.
    static var footprintText: String { size(taskInfo()?.phys_footprint ?? 0) }

    nonisolated private static func taskInfo() -> task_vm_info_data_t? {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
        let status = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return status == KERN_SUCCESS ? info : nil
    }

    /// Plain text to paste in a bug report.
    var text: String {
        var lines = [
            "\(L("ui.debug.footprint")): \(size(footprint)) (\(L("ui.debug.peak")) \(size(peak)), \(L("ui.debug.resident")) \(size(resident)))",
        ]
        lines += kinds.map { "\($0.kind.label): \(size($0.bytes))" }
        lines += [
            "\(L("ui.debug.heap")): \(size(heapInUse)) / \(size(heapAllocated))",
            "\(L("ui.debug.rust_core")): \(size(rustLive)) (\(L("ui.debug.peak")) \(size(rustPeak)), \(rustAllocations))",
            "\(L("ui.debug.avatars")): \(avatars.count), \(size(UInt64(avatars.bytes)))",
            "\(L("ui.debug.previews")): \(previews.count), \(size(UInt64(previews.bytes)))",
            "\(L("ui.debug.disk_cache")): \(size(cache.previewBytes + cache.photoBytes)) (\(L("ui.debug.cache_previews")) \(cache.previewFiles), \(L("ui.debug.cache_photos")) \(cache.photoFiles))",
        ]
        return lines.joined(separator: "\n")
    }
}

private func size(_ bytes: UInt64) -> String {
    ByteCountFormatter.string(fromByteCount: Int64(clamping: bytes), countStyle: .memory)
}

/// Settings › Debug: what uses memory, refreshed every two seconds while open.
struct DebugSettingsSection: View {
    @Environment(Store.self) private var store
    @State private var report: MemoryReport?

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            if let report {
                process(report)
                kinds(report)
                heap(report)
                diskCache(report)
                Button(L("ui.debug.copy_report")) { NSPasteboard.copy(report.text) }
                    .buttonStyle(OutlineButtonStyle())
            } else {
                ProgressView().controlSize(.small)
            }
        }
        .task {
            while !Task.isCancelled {
                await refresh()
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    private func refresh() async {
        var next = await Task.detached(priority: .utility) { MemoryReport.measure() }.value
        (next.avatars, next.previews) = store.imageMemory()
        report = next
    }

    private func process(_ report: MemoryReport) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(L("ui.debug.footprint")).font(.figtree(14, .semibold))
            Text(size(report.footprint)).font(.figtree(28, .semibold)).monospacedDigit()
            Muted(L("ui.debug.footprint_detail", ["peak": size(report.peak), "resident": size(report.resident)]))
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .card(padding: 18)
    }

    private func kinds(_ report: MemoryReport) -> some View {
        let largest = max(report.kinds.first?.bytes ?? 1, 1)
        return VStack(alignment: .leading, spacing: 10) {
            Text(L("ui.debug.breakdown")).font(.figtree(14, .semibold))
            ForEach(report.kinds, id: \.kind) { entry in
                row(entry.kind.label, size(entry.bytes), fraction: Double(entry.bytes) / Double(largest))
            }
            Muted(L("ui.debug.breakdown_detail"))
        }
        .card(padding: 18)
    }

    private func heap(_ report: MemoryReport) -> some View {
        let total = max(report.heapInUse, 1)
        let swift = report.heapInUse > report.rustLive ? report.heapInUse - report.rustLive : 0
        return VStack(alignment: .leading, spacing: 10) {
            Text(L("ui.debug.heap")).font(.figtree(14, .semibold))
            Muted(L("ui.debug.heap_detail", ["used": size(report.heapInUse), "reserved": size(report.heapAllocated)]))
            row(L("ui.debug.rust_core"), size(report.rustLive), fraction: Double(report.rustLive) / Double(total),
                detail: L("ui.debug.rust_detail", ["peak": size(report.rustPeak), "n": report.rustAllocations]))
            row(L("ui.debug.swift_side"), size(swift), fraction: Double(swift) / Double(total))
            row(L("ui.debug.avatars"), size(UInt64(report.avatars.bytes)),
                fraction: Double(report.avatars.bytes) / Double(total),
                detail: L("ui.debug.image_count", ["n": report.avatars.count]))
            row(L("ui.debug.previews"), size(UInt64(report.previews.bytes)),
                fraction: Double(report.previews.bytes) / Double(total),
                detail: L("ui.debug.image_count", ["n": report.previews.count]))
        }
        .card(padding: 18)
    }

    private func diskCache(_ report: MemoryReport) -> some View {
        let cache = report.cache
        let total = max(cache.previewBytes + cache.photoBytes, 1)
        return VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(L("ui.debug.disk_cache")).font(.figtree(14, .semibold))
                Spacer()
                Text(size(cache.previewBytes + cache.photoBytes)).font(.figtree(14, .semibold)).monospacedDigit()
            }
            Muted(L("ui.debug.disk_cache_detail"))
            row(L("ui.debug.cache_previews"), size(cache.previewBytes),
                fraction: Double(cache.previewBytes) / Double(total),
                detail: L("ui.debug.file_count", ["n": cache.previewFiles]))
            row(L("ui.debug.cache_photos"), size(cache.photoBytes),
                fraction: Double(cache.photoBytes) / Double(total),
                detail: L("ui.debug.file_count", ["n": cache.photoFiles]))
            Button(L("ui.debug.clear_cache")) {
                Task {
                    await Task.detached(priority: .utility) { clearPictureCache() }.value
                    await refresh()
                }
            }
            .buttonStyle(OutlineButtonStyle())
            .disabled(cache.previewFiles + cache.photoFiles == 0)
        }
        .card(padding: 18)
    }

    private func row(_ title: String, _ value: String, fraction: Double, detail: String? = nil) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(title).font(.figtree(13))
                if let detail { Muted(detail) }
                Spacer()
                Text(value).font(.figtree(13, .semibold)).monospacedDigit()
            }
            GeometryReader { proxy in
                ZStack(alignment: .leading) {
                    Capsule().fill(Theme.input)
                    Capsule().fill(Theme.accent).frame(width: proxy.size.width * min(max(fraction, 0), 1))
                }
            }
            .frame(height: 5)
        }
        .accessibilityElement(children: .combine)
    }
}
