import UIKit
import Flutter

@main
@objc class AppDelegate: FlutterAppDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    GeneratedPluginRegistrant.register(with: self)

    let registrar = self.registrar(forPlugin: "NimbusMapPlugin")!
    let factory = NimbusMapFactory(messenger: registrar.messenger())
    registrar.register(factory, id: "com.yannicked.nimbus/map_view")

    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }
}
