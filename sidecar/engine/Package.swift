// swift-tools-version:5.10
import PackageDescription

let package = Package(
    name: "engine",
    platforms: [.macOS("15.0")],
    targets: [
        .executableTarget(
            name: "engine",
            path: "Sources/engine"
        )
    ]
)
