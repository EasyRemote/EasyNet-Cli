// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "RuntimeSDK",
    platforms: [
        .macOS(.v14),
        .iOS(.v17)
    ],
    products: [
        .library(
            name: "RuntimeSDK",
            targets: ["RuntimeSDK"]
        )
    ],
    targets: [
        .target(
            name: "RuntimeSDK",
            path: "Sources/RuntimeSDK"
        ),
        .testTarget(
            name: "RuntimeSDKTests",
            dependencies: ["RuntimeSDK"],
            path: "Tests/RuntimeSDKTests"
        )
    ]
)
