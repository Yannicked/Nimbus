package com.yannicked.nimbus

import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine

class MainActivity: FlutterActivity() {
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        flutterEngine
            .platformViewsController
            .registry
            .registerViewFactory(
                "com.yannicked.nimbus/map_view",
                NimbusMapFactory(flutterEngine.dartExecutor.binaryMessenger)
            )
    }
}
