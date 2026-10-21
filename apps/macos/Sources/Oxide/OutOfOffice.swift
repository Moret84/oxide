import SwiftUI
import OxideCore

/// Settings › Out of office: automatic replies, and when they are sent.
struct OutOfOfficePage: View {
    @Environment(Store.self) private var store
    let back: () -> Void
    @State private var draft: OutOfOffice?
    var body: some View {
        let view = store.me?.outOfOfficeSettings
        SettingsSubpage(title: L("ui.out_of_office.title"), back: back) {
            if let binding = Binding($draft) {
                form(binding, view: view)
            } else {
                ProgressView()
            }
            if let error = store.me?.error {
                HStack(alignment: .top) {
                    Text(error).font(.figtree(12)).foregroundStyle(Theme.errorText).textSelection(.enabled)
                    Spacer()
                    IconButton(symbol: "xmark", help: L("ui.status_menu.close"), side: 20, glyph: 10) {
                        store.send(.me(action: .dismissError))
                    }
                }
            }
        }
        .onAppear { store.send(.me(action: .loadOutOfOffice)) }
        .onChange(of: view?.settings) { _, settings in
            if let settings, view?.saving != true { draft = settings }
        }
    }

    private func form(_ draft: Binding<OutOfOffice>, view: OutOfOfficeView?) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            Toggle(L("ui.out_of_office.turn_on"), isOn: draft.enabled).toggleStyle(.switch)
            Muted(L("ui.out_of_office.explanation"), size: 12)
            if draft.wrappedValue.enabled {
                VStack(alignment: .leading, spacing: 6) {
                    Text(L("ui.out_of_office.reply_inside")).font(.figtree(13, .semibold))
                    messageEditor(draft.internalMessage, label: L("ui.out_of_office.reply_inside"))
                }
                Toggle(L("ui.out_of_office.reply_outside"), isOn: draft.replyOutside)
                if draft.wrappedValue.replyOutside {
                    Picker("", selection: draft.outsideAudience) {
                        Text(L("ui.out_of_office.contacts_only")).tag(OutsideAudience.contactsOnly)
                        Text(L("ui.out_of_office.everyone_outside")).tag(OutsideAudience.all)
                    }
                    .pickerStyle(.radioGroup).labelsHidden()
                    messageEditor(draft.externalMessage, label: L("ui.out_of_office.reply_outside_message"))
                }
                Toggle(L("ui.out_of_office.only_during_period"), isOn: draft.scheduled)
                if draft.wrappedValue.scheduled {
                    DatePicker(L("ui.out_of_office.start"), selection: date(draft.startMs))
                    DatePicker(L("ui.out_of_office.end"), selection: date(draft.endMs))
                }
            }
            HStack {
                Button(view?.saving == true ? L("ui.out_of_office.saving") : L("ui.out_of_office.save")) {
                    store.send(.me(action: .saveOutOfOffice(settings: draft.wrappedValue)))
                }
                .buttonStyle(PrimaryButtonStyle())
                .disabled(view?.saving == true || draft.wrappedValue == view?.settings)
                if view?.loading == true { ProgressView().controlSize(.small) }
            }
        }
        .card(padding: 20)
    }

    private func messageEditor(_ text: Binding<String>, label: String) -> some View {
        TextEditor(text: text)
            .font(.figtree(13)).frame(height: 90).padding(4)
            .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Theme.borderStrong))
            .accessibilityLabel(label)
    }

    /// Epoch milliseconds as a date, on the 30-minute grid the web client offers.
    private func date(_ ms: Binding<Int64>) -> Binding<Date> {
        Binding(
            get: { Date(timeIntervalSince1970: TimeInterval(ms.wrappedValue) / 1000) },
            set: {
                let step: TimeInterval = 30 * 60
                let rounded = ($0.timeIntervalSince1970 / step).rounded() * step
                ms.wrappedValue = Int64(rounded * 1000)
            }
        )
    }
}
