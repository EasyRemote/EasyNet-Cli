// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "EasyNetDaemonSDK",
    platforms: [
        .macOS(.v14),
        .iOS(.v17)
    ],
    products: [
        .library(
            name: "EasyNetDaemonSDK",
            targets: ["EasyNetDaemonSDK"]
        )
    ],
    targets: [
        .target(
            name: "EasyNetDaemonSDK",
            path: "Sources/EasyNetDaemonSDK"
        ),
        .testTarget(
            name: "EasyNetDaemonSDKTests",
            dependencies: ["EasyNetDaemonSDK"],
            path: "Tests/EasyNetDaemonSDKTests"
        )
    ]
)
