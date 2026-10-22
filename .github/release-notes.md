**Oxide** {{VERSION}}, an unofficial native Mac client for Microsoft Teams.

## Installation

1. Download `Oxide-{{VERSION}}.zip` below and unzip it.
2. Drag `Oxide.app` into your **Applications** folder.
3. On first launch, macOS will say the app "cannot be verified": it is not notarized by Apple yet. To allow it, either:
   - open **System Settings › Privacy & Security** and click **Open Anyway** near the bottom;
   - or run in Terminal:
     ```bash
     xattr -dr com.apple.quarantine /Applications/Oxide.app
     ```

## Requirements

- macOS 14 (Sonoma) or later
- **Apple Silicon Mac (M1 or later) only**: this build does not run on Intel Macs.

## Known limitations

- The app is signed ad hoc, not with a Developer ID certificate, and is not notarized, hence the Gatekeeper warning above.
- macOS may ask again for Keychain access to your saved session after each update.
- No universal (Intel) build yet.

The installed version is shown at the bottom of the **Settings** page.
