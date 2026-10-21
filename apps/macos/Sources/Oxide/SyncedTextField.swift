import SwiftUI

/// A core-backed text field: the core's echo of typing is ignored, only its own changes (clear, reset) replace the text.
struct SyncedTextField: View {
    let placeholder: String
    let value: String
    var secure = false
    /// `.vertical` grows the field over several lines (Option-Return inserts one).
    var axis: Axis = .horizontal
    let onChange: (String) -> Void
    var onSubmit: () -> Void = {}

    @State private var text = ""
    @State private var sent: [String] = []

    var body: some View {
        Group {
            if secure {
                SecureField(placeholder, text: binding)
            } else {
                TextField(placeholder, text: binding, axis: axis)
            }
        }
        .onSubmit(onSubmit)
        .onAppear { text = value }
        .onChange(of: value) { _, incoming in
            if let index = sent.firstIndex(of: incoming) {
                sent.removeFirst(index + 1)
            } else if incoming != text {
                sent.removeAll()
                text = incoming
            }
        }
    }

    private var binding: Binding<String> {
        Binding(
            get: { text },
            set: { new in
                guard new != text else { return }
                text = new
                sent.append(new)
                onChange(new)
            }
        )
    }
}
