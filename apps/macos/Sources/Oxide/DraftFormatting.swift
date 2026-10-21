import AppKit
import OxideCore

enum InlineFormat { case bold, italic, underline, strike, code }

extension DraftInput {
    func refreshFormat() {
        guard let text = textView else { return }
        let attrs: [NSAttributedString.Key: Any]
        if selection.length > 0, selection.location < (text.textStorage?.length ?? 0) {
            attrs = text.textStorage!.attributes(at: selection.location, effectiveRange: nil)
        } else { attrs = text.typingAttributes }
        currentFormat = ComposerText.format(attrs, range: selection)
    }
    func shortcut(_ key: String) {
        switch key {
        case "b": toggle(.bold)
        case "i": toggle(.italic)
        case "u": toggle(.underline)
        case "strike": toggle(.strike)
        case "k": linkRequested = true
        default: break
        }
    }
    func toggle(_ option: InlineFormat) {
        let key: WritableKeyPath<ComposerFormat, Bool> = switch option {
        case .bold: \.bold
        case .italic: \.italic
        case .underline: \.underline
        case .strike: \.strike
        case .code: \.code
        }
        let enabled = !currentFormat[keyPath: key]
        change { $0[keyPath: key] = enabled }
    }
    func setBlock(_ block: ComposerBlock) {
        let chosen: ComposerBlock = currentFormat.block == block ? .paragraph : block
        change(paragraphs: true) { style in
            style.block = chosen
            switch chosen {
            case .heading1: style.size = 26; style.bold = true
            case .heading2: style.size = 22; style.bold = true
            case .heading3: style.size = 18; style.bold = true
            default: style.size = 0; style.bold = false
            }
        }
    }
    func indent(_ delta: Int) {
        change(paragraphs: true) { $0.indent = UInt32(min(max(Int($0.indent) + delta, 0), 8)) }
    }
    func setAlignment(_ alignment: String) { change(paragraphs: true) { $0.alignment = alignment } }
    func clearFormatting() {
        change { style in
            let range = (style.start, style.length)
            style = ComposerText.empty
            style.start = range.0; style.length = range.1
        }
    }
    func change(paragraphs: Bool = false, _ transform: (inout ComposerFormat) -> Void) {
        guard let text = textView, let storage = text.textStorage, NSMaxRange(selection) <= storage.length else { return }
        text.window?.makeFirstResponder(text)
        text.setSelectedRange(selection)
        let range = paragraphs ? (text.string as NSString).paragraphRange(for: selection) : selection
        if range.length == 0 {
            var style = ComposerText.format(text.typingAttributes, range: range)
            transform(&style)
            text.typingAttributes = ComposerText.attributes(style)
            currentFormat = style
            return
        }
        let before = NSAttributedString(attributedString: storage)
        let savedSelection = selection
        storage.beginEditing()
        before.enumerateAttributes(in: range) { attrs, run, _ in
            var style = ComposerText.format(attrs, range: run)
            transform(&style)
            storage.setAttributes(ComposerText.attributes(style), range: run)
        }
        storage.endEditing()
        text.setSelectedRange(savedSelection)
        registerUndo(before, selection: savedSelection)
        text.didChangeText()
        refreshFormat()
    }
    private func registerUndo(_ attributed: NSAttributedString, selection: NSRange) {
        textView?.undoManager?.registerUndo(withTarget: self) { target in target.restore(attributed, selection: selection) }
        textView?.undoManager?.setActionName(L("ui.draft_formatting.formatting"))
    }
    private func restore(_ attributed: NSAttributedString, selection: NSRange) {
        guard let text = textView, let storage = text.textStorage else { return }
        let before = NSAttributedString(attributedString: storage)
        let previousSelection = text.selectedRange()
        storage.setAttributedString(attributed)
        text.setSelectedRange(selection)
        self.selection = selection
        registerUndo(before, selection: previousSelection)
        text.didChangeText()
        refreshFormat()
    }
    var selectedText: String {
        guard let text = textView, NSMaxRange(selection) <= (text.string as NSString).length else { return "" }
        return (text.string as NSString).substring(with: selection)
    }
    func insertLink(title: String, url: String) {
        guard let link = URL(string: url.trimmingCharacters(in: .whitespacesAndNewlines)), ["https", "http", "mailto"].contains(link.scheme?.lowercased() ?? ""), let text = textView else { return }
        text.window?.makeFirstResponder(text)
        let chosen = title.isEmpty ? link.absoluteString : title
        if chosen != selectedText {
            let start = selection.location
            text.insertText(chosen, replacementRange: selection)
            selection = NSRange(location: start, length: (chosen as NSString).length)
            text.setSelectedRange(selection)
        }
        change { $0.link = link.absoluteString }
    }
    func insertRule() {
        guard let text = textView else { return }
        text.window?.makeFirstResponder(text)
        let start = selection.location
        let prefix = start > 0 && (text.string as NSString).substring(with: NSRange(location: start - 1, length: 1)) != "\n" ? "\n" : ""
        text.insertText(prefix + "────────\n", replacementRange: selection)
        selection = NSRange(location: start + (prefix as NSString).length, length: 8)
        text.setSelectedRange(selection)
        setBlock(.rule)
        selection = NSRange(location: start + (prefix as NSString).length + 9, length: 0)
        text.setSelectedRange(selection)
        text.typingAttributes = ComposerText.base
        refreshFormat()
    }
}
