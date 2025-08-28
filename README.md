# blesync

**Multiplatform library for two-way data synchronization over BLE**

## Overview

**blesync** is designed to simplify Bluetooth Low Energy data synchronization across platforms. It offers:
- A **macOS central** implementation (in Rust)
- An **Android peripheral** implementation (in Kotlin)

This library enables seamless, reliable two-way synchronization of structured data between devices.

---

## Table of Contents

1. [Features](#features)  
2. [Supported Platforms](#supported-platforms)  
3. [Getting Started](#getting-started)  
   - [Prerequisites](#prerequisites)  
   - [Installation & Setup](#installation--setup)  
4. [Usage](#usage)  
   - [macOS (Rust central)](#macos-rust-central)  
   - [Android (Kotlin peripheral)](#android-kotlin-peripheral)  
5. [Synchronizing Data](#synchronizing-data)  
6. [Contributing](#contributing)  
7. [License](#license)  
8. [Acknowledgments](#acknowledgments)

---

## Features

- **Cross-platform BLE synchronization** (macOS ↔ Android)  
- **Two-way data transfer** — both devices can initiate and respond  
- **Core implementations** in Rust (macOS) and Kotlin (Android)  
- Modular architecture — easy to extend to other platforms

---

## Supported Platforms

| Platform | Role       | Language |
|----------|------------|----------|
| macOS    | Central    | Rust     |
| Android  | Peripheral | Kotlin   |

---

## Getting Started

### Prerequisites

- **macOS**: Rust toolchain installed (`rustc`, `cargo`), plus any required BLE permissions or system configurations.  
- **Android**: Android development environment (Android Studio, SDK, emulator/device), plus Bluetooth permissions in the app manifest.

### Installation & Setup

1. Clone the repository:

   ```bash
   git clone https://github.com/jacoberrol/blesync.git
   cd blesync
