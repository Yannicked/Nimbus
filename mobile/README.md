<div align="center">

# 📱 Nimbus Mobile

**Cross-platform companion app for Nimbus, featuring hardware-accelerated radar overlays and interactive meteorological forecasts.**

[![Flutter](https://img.shields.io/badge/Flutter-Android%20%7C%20iOS-02569B?logo=flutter&logoColor=white)](https://flutter.dev/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](../LICENSE)

</div>

---

## 📖 Overview

The **Nimbus Mobile** app brings the high-resolution precipitation ensemble, temperature, and wind forecasts of Nimbus to Android and iOS devices. It features custom OpenGL/WebGL texture overlays for smooth 60fps radar animation and seamless timeline scrubbing on mobile screens.

## 🚀 Getting Started

### Prerequisites

- [Flutter SDK](https://docs.flutter.dev/get-started/install) (3.x+)
- Android Studio / Xcode for device simulation and deployment
- A running [Nimbus Backend Server](../README.md) instance

### Setup & Run

1. **Install dependencies:**
   ```bash
   flutter pub get
   ```

2. **Run in development mode:**
   ```bash
   flutter run
   ```

3. **Build release binaries:**
   ```bash
   # Android APK / App Bundle
   flutter build apk --release
   flutter build appbundle --release

   # iOS
   flutter build ipa --release
   ```

## 🧪 Testing & Quality

```bash
# Check code formatting
dart format --output=none --set-exit-if-changed .

# Run static analysis
flutter analyze

# Run unit tests
flutter test
```

## 📜 License

Distributed under the **MIT License**. See [`LICENSE`](../LICENSE) for more information.
