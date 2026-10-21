import SwiftUI
import OxideCore

struct SignInView: View {
    @Environment(Store.self) private var store
    let shell: Shell

    var body: some View {
        HStack(spacing: 0) {
            BrandPanel().frame(width: 440)
            VStack(spacing: 0) {
                HStack {
                    Spacer()
                    ThemeSwitcher()
                }
                .padding(20)
                Spacer()
                form
                Spacer()
                Muted(L("ui.sign_in_view.your_sign_in_tokens_stay_in"), size: 11).padding(20)
            }
            .frame(maxWidth: .infinity)
        }
    }

    private var form: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(L("ui.sign_in_view.welcome")).font(.figtree(28, .semibold))
            Text(L("ui.sign_in_view.sign_in_with_your_microsoft_work"))
                .font(.figtree(14))
                .foregroundStyle(Theme.muted)
                .lineSpacing(3)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 8)
            Button {
                store.send(.signIn)
            } label: {
                HStack(spacing: 10) {
                    if shell.signingIn {
                        ProgressView().controlSize(.small)
                    } else {
                        MicrosoftLogo()
                    }
                    Text(shell.signingIn ? L("ui.sign_in_view.signing_in") : L("ui.sign_in_view.sign_in_with_microsoft"))
                }
                .frame(maxWidth: .infinity)
            }
            .buttonStyle(SignInButtonStyle())
            .disabled(shell.signingIn)
            .keyboardShortcut(.defaultAction)
            .padding(.top, 28)
            if let error = shell.error {
                Text(error)
                    .font(.figtree(12))
                    .foregroundStyle(Theme.errorText)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Theme.errorBackground, in: RoundedRectangle(cornerRadius: 8))
                    .textSelection(.enabled)
                    .padding(.top, 16)
            }
            Muted(L("ui.sign_in_view.you_sign_in_on_microsoft_s"), size: 12)
                .lineSpacing(2)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 16)
        }
        .frame(width: 360)
    }
}

/// The violet half of the sign-in screen; the same in light and dark mode.
private struct BrandPanel: View {
    var body: some View {
        ZStack(alignment: .topLeading) {
            LinearGradient(colors: [Theme.brandTop, Theme.brandBottom], startPoint: .topLeading, endPoint: .bottomTrailing)
            OxideMark(bubble: [.white], people: false)
                .opacity(0.07)
                .frame(width: 420, height: 400)
                .offset(x: 170, y: 420)
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 10) {
                    OxideMark(bubble: [.white], figure: Theme.brandBottom).frame(width: 30, height: 28)
                    Text("Oxide").font(.figtree(19, .semibold))
                }
                Spacer()
                Text(L("ui.sign_in_view.microsoft_teams_native_on_your_mac"))
                    .font(.figtree(30, .semibold))
                    .lineSpacing(4)
                    .fixedSize(horizontal: false, vertical: true)
                VStack(alignment: .leading, spacing: 14) {
                    feature("bubble.left.and.bubble.right", L("ui.sign_in_view.chats_teams_and_channels"))
                    feature("video", L("ui.sign_in_view.calls_and_meetings"))
                    feature("calendar", L("ui.sign_in_view.calendar_and_files"))
                }
                .padding(.top, 28)
                Spacer()
                Text(L("ui.sign_in_view.independent_client_not_affiliated_with_microsoft"))
                    .font(.figtree(11))
                    .opacity(0.65)
            }
            .padding(40)
        }
        .foregroundStyle(.white)
        .clipped()
    }

    private func feature(_ symbol: String, _ text: String) -> some View {
        HStack(spacing: 12) {
            Image(systemName: symbol)
                .font(.system(size: 13, weight: .medium))
                .frame(width: 30, height: 30)
                .background(.white.opacity(0.16), in: RoundedRectangle(cornerRadius: 8))
            Text(text).font(.figtree(14)).opacity(0.92)
        }
    }
}

/// Système / Clair / Sombre as three icons, writing the same setting as Settings › Apparence.
private struct ThemeSwitcher: View {
    @AppStorage(Appearance.storageKey) private var appearance = Appearance.system

    var body: some View {
        HStack(spacing: 2) {
            ForEach(Appearance.allCases) { option in
                let active = option == appearance
                Button { appearance = option } label: {
                    Image(systemName: symbol(option))
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(active ? Theme.accentText : Theme.muted)
                        .frame(width: 30, height: 24)
                        .background(active ? Theme.accentSoft : .clear, in: RoundedRectangle(cornerRadius: 6))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .pointerCursor()
                .help(L("ui.sign_in_view.theme", ["theme": option.label]))
                .accessibilityLabel(L("ui.sign_in_view.theme", ["theme": option.label]))
                .accessibilityAddTraits(active ? .isSelected : [])
            }
        }
        .padding(3)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.border))
    }

    private func symbol(_ option: Appearance) -> String {
        switch option {
        case .system: "circle.lefthalf.filled"
        case .light: "sun.max"
        case .dark: "moon"
        }
    }
}

/// Neutral button in the style of Microsoft's "Sign in with Microsoft".
private struct SignInButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.figtree(14, .semibold))
            .foregroundStyle(Theme.text)
            .frame(minHeight: 42)
            .background(configuration.isPressed ? Theme.input : Theme.surface, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.borderStrong))
            .opacity(enabled ? 1 : 0.7)
            .pointerCursor()
    }
}

private struct MicrosoftLogo: View {
    var body: some View {
        Grid(horizontalSpacing: 1.5, verticalSpacing: 1.5) {
            GridRow { square(0xF25022); square(0x7FBA00) }
            GridRow { square(0x00A4EF); square(0xFFB900) }
        }
        .accessibilityHidden(true)
    }
    private func square(_ hex: UInt32) -> some View {
        Theme.rgb(hex).frame(width: 7.5, height: 7.5)
    }
}
