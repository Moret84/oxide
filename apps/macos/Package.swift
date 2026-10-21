// swift-tools-version:6.0
// Native macOS shell over the Rust core. Build the core first: scripts/build-core.sh
import PackageDescription

let package = Package(
    name: "Oxide",
    platforms: [.macOS(.v14)],
    dependencies: [
        // libwebrtc with macOS audio device selection; symbols are prefixed LKRTC.
        .package(url: "https://github.com/livekit/webrtc-xcframework", exact: "144.7559.15"),
    ],
    targets: [
        .binaryTarget(name: "OxideCoreFFI", path: "OxideCore.xcframework"),
        .target(
            name: "OxideCore",
            dependencies: ["OxideCoreFFI"],
            linkerSettings: [
                .linkedFramework("Security"),
                .linkedFramework("SystemConfiguration"),
                .linkedFramework("CoreFoundation"),
            ]
        ),
        .executableTarget(
            name: "Oxide",
            dependencies: ["OxideCore", .product(name: "LiveKitWebRTC", package: "webrtc-xcframework")]
        ),
        .testTarget(name: "OxideTests", dependencies: ["Oxide", "OxideCore"]),
    ],
    swiftLanguageModes: [.v5]
)
