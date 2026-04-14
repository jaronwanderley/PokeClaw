// swift-tools-version:5.3
import PackageDescription

let package = Package(
    name: "tauri-plugin-pokeclaw",
    platforms: [
        .iOS(.v13)
    ],
    products: [
        .library(
            name: "tauri-plugin-pokeclaw",
            type: .static,
            targets: ["PokeclawPlugin"])
    ],
    dependencies: [
        .package(name: "Tauri", url: "https://github.com/tauri-apps/tauri-plugin-ios", .branch("main"))
    ],
    targets: [
        .target(
            name: "PokeclawPlugin",
            dependencies: ["Tauri"],
            path: "Sources")
    ]
)
