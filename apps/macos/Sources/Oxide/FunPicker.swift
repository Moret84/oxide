import ImageIO
import SwiftUI
import OxideCore

/// The composer's GIF and sticker picker, as the web client's "fun stuff" panel.
struct FunPicker: View {
    @Environment(Store.self) private var store
    let close: () -> Void
    @State private var tab = Tab.gifs
    @State private var query = ""
    @State private var category: String?
    private enum Tab: String, Identifiable { case gifs, stickers; var id: String { rawValue } }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                SegmentedPicker(selection: $tab, options: [.gifs, .stickers]) {
                    $0 == .gifs ? L("ui.fun_picker.gif") : L("ui.fun_picker.stickers")
                }
                Spacer()
                if store.gifs?.sending == true { ProgressView().controlSize(.small) }
            }
            FileSearchField(placeholder: tab == .gifs ? L("ui.fun_picker.search_giphy") : L("ui.fun_picker.search_stickers"),
                            current: query, delay: tab == .gifs ? .milliseconds(400) : .zero) {
                query = $0
                if tab == .gifs { store.send(.gifs(action: .search(query: $0))) }
            }
            switch tab {
            case .gifs: gifs
            case .stickers: stickers
            }
        }
        .padding(12)
        .frame(width: 380, height: 420)
        .onAppear { if store.gifs?.results.isEmpty ?? true { store.send(.gifs(action: .search(query: ""))) } }
        .onChange(of: tab) { _, _ in query = "" }
    }

    private var gifs: some View {
        let view = store.gifs
        return ScrollView {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 110), spacing: 6)], spacing: 6) {
                ForEach(view?.results ?? [], id: \.url) { gif in
                    Button {
                        store.send(.gifs(action: .sendGif(url: gif.url)))
                        close()
                    } label: {
                        RemoteThumbnail(url: gif.preview).frame(height: 88)
                    }
                    .buttonStyle(.plain).pointerCursor()
                    .help(gif.title)
                    .accessibilityLabel(gif.title.isEmpty ? L("ui.fun_picker.gif") : gif.title)
                }
            }
            if view?.loading == true { ProgressView().padding() }
            if let error = view?.error { Muted(error, size: 12).padding() }
            if view?.loading == false && view?.results.isEmpty == true && view?.error == nil {
                Muted(L("ui.fun_picker.no_gifs"), size: 12).padding()
            }
            Muted(L("ui.fun_picker.powered_by_giphy"), size: 10).padding(.top, 4)
        }
    }

    private var stickers: some View {
        let all = store.stickers
        let filtered: [StickerView] = query.isEmpty
            ? (all.first(where: { $0.id == (category ?? all.first?.id) })?.stickers ?? [])
            : all.flatMap(\.stickers).filter { $0.name.localizedStandardContains(query) || $0.keywords.localizedStandardContains(query) }
        return VStack(alignment: .leading, spacing: 8) {
            if all.isEmpty {
                HStack { ProgressView().controlSize(.small); Muted(L("ui.fun_picker.loading_stickers"), size: 12) }
            }
            if query.isEmpty && !all.isEmpty {
                Picker("", selection: Binding(get: { category ?? all.first?.id ?? "" }, set: { category = $0 })) {
                    ForEach(all, id: \.id) { Text($0.id).tag($0.id) }
                }
                .labelsHidden()
            }
            ScrollView {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 80), spacing: 6)], spacing: 6) {
                    ForEach(filtered, id: \.id) { sticker in
                        Button {
                            store.send(.gifs(action: .sendSticker(id: sticker.id)))
                            close()
                        } label: {
                            RemoteThumbnail(url: sticker.url).frame(width: 80, height: 80)
                        }
                        .buttonStyle(.plain).pointerCursor()
                        .help(sticker.name)
                        .accessibilityLabel(sticker.name)
                    }
                }
            }
        }
    }
}

/// A public CDN image (Giphy, Teams statics), loaded without credentials.
private struct RemoteThumbnail: View {
    let url: String
    @State private var loaded: NSImage?
    @State private var failed = false
    var body: some View {
        Group {
            if let image = loaded ?? RemoteThumbnails.cached(url) {
                Image(nsImage: image).resizable().scaledToFit()
            } else if failed {
                Image(systemName: "photo").foregroundStyle(Theme.subtle)
            } else {
                ProgressView().controlSize(.small)
            }
        }
        .frame(maxWidth: .infinity)
        .background(Theme.input, in: RoundedRectangle(cornerRadius: 6))
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .task(id: url) {
            failed = false
            loaded = await RemoteThumbnails.load(url)
            failed = loaded == nil
        }
    }
}

/// Thumbnails decoded at grid size and kept in memory, so scrolling or reopening the picker does not decode again.
@MainActor
private enum RemoteThumbnails {
    private static let cache: NSCache<NSString, NSImage> = {
        let cache = NSCache<NSString, NSImage>()
        cache.totalCostLimit = 30 << 20
        return cache
    }()

    static func cached(_ url: String) -> NSImage? { cache.object(forKey: url as NSString) }

    static func load(_ url: String) async -> NSImage? {
        if let image = cached(url) { return image }
        guard let link = URL(string: url), let (data, _) = try? await URLSession.shared.data(from: link),
              let thumbnail = await Task.detached(operation: { decode(data) }).value else { return nil }
        let image = NSImage(cgImage: thumbnail, size: .zero)
        cache.setObject(image, forKey: url as NSString, cost: thumbnail.bytesPerRow * thumbnail.height)
        return image
    }

    /// The first frame, downsampled to at most 220 px.
    nonisolated private static func decode(_ data: Data) -> CGImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: 220,
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }
}
