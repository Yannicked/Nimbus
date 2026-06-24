import Flutter
import UIKit
import Mapbox // MapLibre iOS SDK uses the Mapbox module name or MapLibre depending on package, MGLMapView is standard.
import MetalKit

class NimbusMapView: NSObject, FlutterPlatformView, MGLMapViewDelegate {
    
    private var containerView: UIView
    private var mapView: MGLMapView
    private var overlay: RadarMetalOverlay
    private var channel: FlutterMethodChannel
    
    private var activeLayerMode = "rain"
    private var activeEnsemble = "med"
    private var activeWindHeight = 10
    private var activeTimeIndex = 0
    private var activeVersion = ""
    private var currentRequestSeq = 0

    init(
        frame: CGRect,
        viewId: Int64,
        creationParams: [String: Any]?,
        binaryMessenger: FlutterBinaryMessenger
    ) {
        containerView = UIView(frame: frame)
        
        // 1. Initialize MGLMapView (MapLibre iOS SDK)
        mapView = MGLMapView(frame: frame)
        mapView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        
        // Dark basemap CartoDB style URL
        mapView.styleURL = URL(string: "https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json")
        mapView.setCenter(CLLocationCoordinate2D(latitude: 52.1, longitude: 5.2), zoomLevel: 6.5, animated: false)
        containerView.addSubview(mapView)
        
        // 2. Initialize transparent Metal overlay
        let device = MTLCreateSystemDefaultDevice()!
        overlay = RadarMetalOverlay(frame: frame, device: device)
        overlay.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        containerView.addSubview(overlay)
        
        // Setup Method Channel
        channel = FlutterMethodChannel(
            name: "com.yannicked.nimbus/map_control_\(viewId)",
            binaryMessenger: binaryMessenger
        )
        
        super.init()
        mapView.delegate = self
        
        channel.setMethodCallHandler { [weak self] call, result in
            self?.handle(call, result: result)
        }
        
        // Set tap gesture recognizer for clicks
        let tap = UITapGestureRecognizer(target: self, action: #selector(handleMapTap(_:)))
        mapView.addGestureRecognizer(tap)
    }
    
    func view() -> UIView {
        return containerView
    }
    
    @objc private func handleMapTap(_ sender: UITapGestureRecognizer) {
        let point = sender.location(in: mapView)
        let coord = mapView.convert(point, toCoordinateFrom: mapView)
        
        let args = ["lat": coord.latitude, "lon": coord.longitude]
        channel.invokeMethod("onMapClick", arguments: args)
    }
    
    func mapViewRegionIsChanging(_ mapView: MGLMapView) {
        syncOverlayProjection()
    }
    
    func mapView(_ mapView: MGLMapView, regionDidChangeAnimated animated: Bool) {
        syncOverlayProjection()
    }
    
    private func syncOverlayProjection() {
        // Project coordinates to screen points:
        // Bottom-Left: (48.8526, 0.0), Top-Right: (56.0028, 10.8715)
        let blScreen = mapView.convert(CLLocationCoordinate2D(latitude: 48.8526, longitude: 0.0), toPointTo: mapView)
        let brScreen = mapView.convert(CLLocationCoordinate2D(latitude: 48.8526, longitude: 10.8715), toPointTo: mapView)
        let tlScreen = mapView.convert(CLLocationCoordinate2D(latitude: 56.0028, longitude: 0.0), toPointTo: mapView)
        let trScreen = mapView.convert(CLLocationCoordinate2D(latitude: 56.0028, longitude: 10.8715), toPointTo: mapView)
        
        overlay.updateProjection(
            bl: simd_float2(Float(blScreen.x), Float(blScreen.y)),
            br: simd_float2(Float(brScreen.x), Float(brScreen.y)),
            tl: simd_float2(Float(tlScreen.x), Float(tlScreen.y)),
            tr: simd_float2(Float(trScreen.x), Float(trScreen.y))
        )
    }
    
    func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
        if call.method == "syncState" {
            guard let args = call.arguments as? [String: Any] else {
                result(FlutterError(code: "BAD_ARGS", message: "Arguments must be a Map", details: nil))
                return
            }
            
            activeLayerMode = args["layerMode"] as? String ?? "rain"
            activeEnsemble = args["ensemble"] as? String ?? "med"
            activeWindHeight = args["windHeight"] as? Int ?? 10
            activeTimeIndex = args["currentTimeIndex"] as? Int ?? 0
            overlay.opacity = (args["opacity"] as? NSNumber)?.floatValue ?? 0.7
            let timeVal = args["timeVal"] as? Int ?? 0
            activeVersion = args["version"] as? String ?? ""
            let prefetchTimeVals = args["prefetchTimeVals"] as? [Int] ?? []
            
            // Map layer mode value to shader modes:
            // 0 = rain, 1 = temp, 2 = solar, 3 = wind
            var modeInt: Float = 0.0
            if activeLayerMode == "temp" {
                modeInt = 1.0
            } else if activeLayerMode == "solar" {
                modeInt = 2.0
            } else if activeLayerMode == "wind" {
                modeInt = 3.0
            }
            overlay.layerMode = modeInt
            
            if modeInt == 3.0 {
                overlay.isPaused = false
                overlay.enableSetNeedsDisplay = false
            } else {
                overlay.isPaused = true
                overlay.enableSetNeedsDisplay = true
            }
            
            setupColorStops()
            fetchTextures(timeVal, prefetchTimeVals: prefetchTimeVals)
            result(nil)
        } else {
            result(FlutterMethodNotImplemented)
        }
    }
    
    private func setupColorStops() {
        var values = [Float](repeating: 0, count: 8)
        var colors = [simd_float4](repeating: simd_float4(0, 0, 0, 0), count: 8)
        
        if activeLayerMode == "temp" {
            values = [-10.0, 0.0, 10.0, 20.0, 25.0, 30.0, 35.0, 40.0]
            colors = [
                simd_float4(0/255, 43/255, 128/255, 0.8),
                simd_float4(0/255, 204/255, 255/255, 0.8),
                simd_float4(0/255, 255/255, 102/255, 0.8),
                simd_float4(255/255, 255/255, 0/255, 0.8),
                simd_float4(255/255, 153/255, 0/255, 0.85),
                simd_float4(255/255, 77/255, 77/255, 0.9),
                simd_float4(204/255, 0/255, 0/255, 0.95),
                simd_float4(153/255, 0/255, 77/255, 1.0)
            ]
        } else if activeLayerMode == "solar" {
            values = [10.0, 100.0, 250.0, 500.0, 750.0, 1000.0, 1000.0, 1000.0]
            colors = [
                simd_float4(0/255, 0/255, 0/255, 0.0),
                simd_float4(253/255, 224/255, 71/255, 0.3),
                simd_float4(250/255, 204/255, 21/255, 0.5),
                simd_float4(234/255, 179/255, 8/255, 0.7),
                simd_float4(249/255, 115/255, 22/255, 0.85),
                simd_float4(239/255, 68/255, 68/255, 0.95),
                simd_float4(239/255, 68/255, 68/255, 0.95),
                simd_float4(239/255, 68/255, 68/255, 0.95)
            ]
        } else if activeLayerMode == "wind" {
            values = [0.0, 2.0, 5.0, 10.0, 15.0, 20.0, 25.0, 25.0]
            colors = [
                simd_float4(96/255, 165/255, 250/255, 0.02),
                simd_float4(34/255, 211/255, 238/255, 0.35),
                simd_float4(74/255, 222/255, 128/255, 0.55),
                simd_float4(250/255, 204/255, 21/255, 0.7),
                simd_float4(251/255, 146/255, 60/255, 0.8),
                simd_float4(248/255, 113/255, 113/255, 0.85),
                simd_float4(236/255, 72/255, 153/255, 0.9),
                simd_float4(236/255, 72/255, 153/255, 0.9)
            ]
        } else if activeEnsemble == "prob" {
            values = [0.10, 0.30, 0.50, 0.70, 0.90, 1.00, 1.00, 1.00]
            colors = [
                simd_float4(180/255, 200/255, 220/255, 0.0),
                simd_float4(100/255, 160/255, 255/255, 0.5),
                simd_float4(0/255, 100/255, 255/255, 0.65),
                simd_float4(0/255, 200/255, 100/255, 0.75),
                simd_float4(220/255, 0/255, 220/255, 0.85),
                simd_float4(255/255, 255/255, 255/255, 0.95),
                simd_float4(255/255, 255/255, 255/255, 0.95),
                simd_float4(255/255, 255/255, 255/255, 0.95)
            ]
        } else if activeEnsemble == "spread" {
            values = [0.05, 0.2, 1.0, 5.0, 15.0, 30.0, 30.0, 30.0]
            colors = [
                simd_float4(99/255, 102/255, 241/255, 0.0),
                simd_float4(99/255, 102/255, 241/255, 0.4),
                simd_float4(168/255, 85/255, 247/255, 0.6),
                simd_float4(236/255, 72/255, 153/255, 0.75),
                simd_float4(244/255, 63/255, 94/255, 0.9),
                simd_float4(255/255, 255/255, 255/255, 0.95),
                simd_float4(255/255, 255/255, 255/255, 0.95),
                simd_float4(255/255, 255/255, 255/255, 0.95)
            ]
        } else {
            values = [0.05, 0.2, 1.0, 5.0, 15.0, 30.0, 100.0, 250.0]
            colors = [
                simd_float4(120/255, 200/255, 255/255, 0.0),
                simd_float4(0/255, 100/255, 255/255, 0.7),
                simd_float4(0/255, 200/255, 0/255, 0.7),
                simd_float4(255/255, 230/255, 0/255, 0.8),
                simd_float4(255/255, 120/255, 0/255, 0.9),
                simd_float4(255/255, 0/255, 0/255, 0.95),
                simd_float4(200/255, 0/255, 200/255, 1.0),
                simd_float4(255/255, 255/255, 255/255, 1.0)
            ]
        }
        
        overlay.setStops(values: values, colors: colors)
    }
    
    private func getRelativePath(_ timeVal: Int) -> String {
        switch activeLayerMode {
        case "temp":
            return "/api/data/temp/\(timeVal)"
        case "solar":
            return "/api/data/solar/\(timeVal)"
        case "wind":
            return "/api/data/wind/\(activeWindHeight)/\(timeVal)"
        default:
            return "/api/data/\(activeEnsemble)/\(timeVal)"
        }
    }
    
    private func fetchTextures(_ timeVal: Int, prefetchTimeVals: [Int]) {
        currentRequestSeq += 1
        let seq = currentRequestSeq
        
        let prefs = UserDefaults.standard
        let baseUrl = prefs.string(forKey: "flutter.nimbus_api_base_url") ?? ""
        if baseUrl.isEmpty { return }
        
        let activePath = getRelativePath(timeVal)
        let activeUrl = "\(baseUrl)\(activePath)?v=\(activeVersion)"
        
        // Build set of keepUrls
        var keepUrls = Set<String>()
        keepUrls.insert(activeUrl)
        
        var prefetchUrls: [String] = []
        for pVal in prefetchTimeVals {
            let path = getRelativePath(pVal)
            let url = "\(baseUrl)\(path)?v=\(activeVersion)"
            keepUrls.insert(url)
            prefetchUrls.append(url)
        }
        
        overlay.cancelStaleRequests(keepUrls: keepUrls)
        
        overlay.loadTextureAsync(urlStr: activeUrl, seq: seq)
        
        // Prefetch future frames
        for url in prefetchUrls {
            overlay.prefetchTexture(urlStr: url)
        }
    }
}
