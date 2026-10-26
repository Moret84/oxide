import SwiftUI
import OxideCore

/// A link's preview, as Teams shows it: the page's picture, title, description and site; a click opens the link.
/// Under a message it is a card; in the compose box a single row that can be closed.
struct LinkPreviewCard: View {
    @Environment(Store.self) private var store
    let preview: LinkPreviewView
    /// Set in the compose box: the preview can be left out of the message.
    var onDismiss: (() -> Void)? = nil

    private var compact: Bool { onDismiss != nil }

    var body: some View {
        Group {
            if compact {
                HStack(alignment: .top, spacing: 10) {
                    if preview.loading {
                        ProgressView().controlSize(.small).frame(width: 56, height: 56)
                    } else if let image = preview.image {
                        picture(image).frame(width: 56, height: 56).clipShape(RoundedRectangle(cornerRadius: 6))
                    }
                    texts.frame(maxWidth: .infinity, alignment: .leading)
                    if let onDismiss {
                        IconButton(symbol: "xmark", help: L("ui.link_preview.remove_preview"), side: 22, glyph: 10, action: onDismiss)
                    }
                }
                .padding(8)
                .background(Theme.input, in: RoundedRectangle(cornerRadius: 8))
            } else {
                VStack(alignment: .leading, spacing: 0) {
                    if let image = preview.image {
                        picture(image).frame(height: 150).frame(maxWidth: .infinity).clipped()
                    }
                    texts.padding(.horizontal, 12).padding(.vertical, 10)
                }
                .clipShape(RoundedRectangle(cornerRadius: 10))
                .card(radius: 10)
            }
        }
        .contentShape(Rectangle())
        .onTapGesture { store.send(.openUrl(url: preview.url)) }
        .pointerCursor()
        .help(preview.url)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isLink)
        .accessibilityAction { store.send(.openUrl(url: preview.url)) }
    }

    private var texts: some View {
        VStack(alignment: .leading, spacing: 3) {
            if let title = preview.title {
                Text(title).font(.figtree(13, .semibold)).lineLimit(2)
            }
            if let description = preview.description {
                Muted(description).lineLimit(compact ? 1 : 3)
            }
            HStack(spacing: 4) {
                Image(systemName: "link").font(.system(size: 10)).foregroundStyle(Theme.muted)
                Muted(preview.site, size: 11).lineLimit(1)
            }
        }
    }

    /// Filled to its box; while it loads a spinner, and if it failed why, with a retry.
    @ViewBuilder private func picture(_ image: ImageView) -> some View {
        if case .ready = image.status, let picture = store.imageBox(image.src).image {
            Image(nsImage: picture).resizable().interpolation(.medium).scaledToFill()
                .accessibilityHidden(true)
        } else {
            ZStack {
                Rectangle().fill(Theme.input)
                switch image.status {
                case .loading:
                    ProgressView().controlSize(.small)
                case .failed(let reason) where compact:
                    Button { store.send(.retryImage(src: image.src)) } label: {
                        Image(systemName: "photo").foregroundStyle(Theme.muted)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                    .buttonStyle(.plain)
                    .help("\(reason) · \(L("ui.message_row.retry_image"))")
                case .failed(let reason):
                    VStack(spacing: 6) {
                        Muted(reason, size: 11).multilineTextAlignment(.center).lineLimit(3)
                        Button(L("ui.message_row.retry_image")) { store.send(.retryImage(src: image.src)) }
                            .buttonStyle(GhostButtonStyle())
                            .font(.figtree(12))
                    }
                    .padding(10)
                case .ready:
                    EmptyView()
                }
            }
        }
    }
}
