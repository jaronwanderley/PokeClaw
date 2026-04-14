// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "tauri-plugin-pokeclaw",
    platforms: [
        .iOS(.v16)
    ],
    products: [
        .library(
            name: "tauri-plugin-pokeclaw",
            type: .static,
            targets: ["PokeclawPlugin"])
    ],
    dependencies: [
        .package(name: "Tauri", url: "https://github.com/tauri-apps/tauri-plugin-ios", .branch("main")),
        .package(url: "https://github.com/huggingface/swift-transformers", from: "1.0.0")
    ],
    targets: [
        .target(
            name: "PokeclawPlugin",
            dependencies: [
                "Tauri",
                .product(name: "Transformers", package: "swift-transformers"),
                .product(name: "Hub", package: "swift-transformers")
            ],
            path: "Sources")
    ]
)
