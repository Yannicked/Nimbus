package com.yannicked.nimbus

import android.content.Context
import android.graphics.PixelFormat
import android.opengl.GLSurfaceView
import android.view.View
import android.widget.FrameLayout
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugin.platform.PlatformView
import org.maplibre.android.MapLibre
import org.maplibre.android.geometry.LatLng
import org.maplibre.android.maps.MapView
import org.maplibre.android.maps.MapLibreMap
import org.maplibre.android.maps.OnMapReadyCallback
import org.maplibre.android.maps.Style
import io.flutter.plugin.common.BinaryMessenger

class NimbusMapView(
    private val context: Context,
    viewId: Int,
    creationParams: Map<String, Any?>?,
    messenger: BinaryMessenger
) : PlatformView, MethodChannel.MethodCallHandler, OnMapReadyCallback {

    private val container = FrameLayout(context)
    private val mapView: MapView
    private var map: MapLibreMap? = null
    private val channel = MethodChannel(
        messenger,
        "com.yannicked.nimbus/map_control_$viewId"
    )

    private val glSurfaceView: GLSurfaceView
    private val overlay: RadarGLOverlay

    private var activeServerUrl = ""
    private var activeLayerMode = "rain"
    private var activeEnsemble = "med"
    private var activeWindHeight = 10
    private var activeTimeIndex = 0
    private var activeVersion = ""
    private var currentRequestSeq = 0

    init {
        // Initialize MapLibre Native SDK
        MapLibre.getInstance(context)

        // 1. Create native MapView
        mapView = MapView(context)
        mapView.onCreate(null)
        mapView.getMapAsync(this)
        container.addView(mapView)

        // 2. Create native transparent GLSurfaceView overlay
        glSurfaceView = GLSurfaceView(context)
        glSurfaceView.setEGLContextClientVersion(2)
        glSurfaceView.setEGLConfigChooser(8, 8, 8, 8, 16, 0)
        
        overlay = RadarGLOverlay(context)
        overlay.glSurfaceView = glSurfaceView
        glSurfaceView.setRenderer(overlay)
        glSurfaceView.renderMode = GLSurfaceView.RENDERMODE_WHEN_DIRTY
        glSurfaceView.holder.setFormat(PixelFormat.TRANSLUCENT)
        glSurfaceView.setZOrderMediaOverlay(true)
        container.addView(glSurfaceView)

        // Configure standard method channel receiver
        channel.setMethodCallHandler(this)
    }

    override fun getView(): View {
        return container
    }

    override fun onMapReady(mapboxMap: MapLibreMap) {
        this.map = mapboxMap

        // Set default camera position (centered on Netherlands)
        mapboxMap.setCameraPosition(
            org.maplibre.android.camera.CameraPosition.Builder()
                .target(LatLng(52.1, 5.2))
                .zoom(6.5)
                .build()
        )

        // Load CartoDB Dark Matter base vector tiles style
        mapboxMap.setStyle(
            Style.Builder().fromUri("https://basemaps.cartocdn.com/gl/dark-matter-gl-style/style.json")
        ) { style ->
            // Styles fully loaded
        }

        // Set map gesture listeners
        mapboxMap.addOnMapClickListener { latLng ->
            val args = mapOf("lat" to latLng.latitude, "lon" to latLng.longitude)
            channel.invokeMethod("onMapClick", args)
            true
        }

        // Sync OpenGL coordinates overlay on map camera changes
        mapboxMap.addOnCameraMoveListener {
            syncOverlayProjection()
        }
        mapboxMap.addOnCameraIdleListener {
            syncOverlayProjection()
        }

        // Sync on every frame rendering lifecycle to prevent lag/slipping during map gestures
        mapView.addOnWillStartRenderingFrameListener {
            syncOverlayProjection()
        }
    }

    private fun syncOverlayProjection() {
        val mapInstance = map ?: return
        val projection = mapInstance.projection

        // Bounding box corners: BL = (48.8526, 0.0), TR = (56.0028, 10.8715)
        val bl = projection.toScreenLocation(LatLng(48.8526, 0.0))
        val br = projection.toScreenLocation(LatLng(48.8526, 10.8715))
        val tl = projection.toScreenLocation(LatLng(56.0028, 0.0))
        val tr = projection.toScreenLocation(LatLng(56.0028, 10.8715))

        overlay.updateProjection(
            bl.x, bl.y,
            br.x, br.y,
            tl.x, tl.y,
            tr.x, tr.y
        )

        glSurfaceView.requestRender()
    }

    override fun onMethodCall(call: MethodCall, result: MethodChannel.Result) {
        if (call.method == "syncState") {
            val args = call.arguments as? Map<String, Any?>
            if (args != null) {
                activeLayerMode = args["layerMode"] as? String ?: "rain"
                activeEnsemble = args["ensemble"] as? String ?: "med"
                activeWindHeight = (args["windHeight"] as? Number)?.toInt() ?: 10
                activeTimeIndex = (args["currentTimeIndex"] as? Number)?.toInt() ?: 0
                val opacityVal = (args["opacity"] as? Number)?.toFloat() ?: 0.7f
                val timeVal = (args["timeVal"] as? Number)?.toInt() ?: 0
                activeVersion = args["version"] as? String ?: ""

                overlay.opacity = opacityVal

                // Map layer mode value to shader modes:
                // 0 = rain, 1 = temp, 2 = solar, 3 = wind
                var modeInt = 0f
                if (activeLayerMode == "temp") {
                    modeInt = 1f
                } else if (activeLayerMode == "solar") {
                    modeInt = 2f
                } else if (activeLayerMode == "wind") {
                    modeInt = 3f
                }
                overlay.layerMode = modeInt

                if (activeLayerMode == "wind") {
                    glSurfaceView.renderMode = GLSurfaceView.RENDERMODE_CONTINUOUSLY
                } else {
                    glSurfaceView.renderMode = GLSurfaceView.RENDERMODE_WHEN_DIRTY
                }

                val prefetchTimeVals = args["prefetchTimeVals"] as? List<*> ?: emptyList<Any>()

                // Configure stop limits and colors based on active layers
                setupColorStops()

                // Trigger network loading of WebP textures
                fetchTextures(timeVal, prefetchTimeVals)
                glSurfaceView.requestRender()
                result.success(null)
            } else {
                result.error("BAD_ARGS", "Missing arguments map", null)
            }
        } else {
            result.notImplemented()
        }
    }

    private fun setupColorStops() {
        val values: FloatArray
        val colors: FloatArray

        if (activeLayerMode == "temp") {
            values = floatArrayOf(-10.0f, 0.0f, 10.0f, 20.0f, 25.0f, 30.0f, 35.0f, 40.0f)
            colors = floatArrayOf(
                0f/255f, 43f/255f, 128f/255f, 0.8f,
                0f/255f, 204f/255f, 255f/255f, 0.8f,
                0f/255f, 255f/255f, 102f/255f, 0.8f,
                255f/255f, 255f/255f, 0f/255f, 0.8f,
                255f/255f, 153f/255f, 0f/255f, 0.85f,
                255f/255f, 77f/255f, 77f/255f, 0.9f,
                204f/255f, 0f/255f, 0f/255f, 0.95f,
                153f/255f, 0f/255f, 77f/255f, 1.0f
            )
        } else if (activeLayerMode == "solar") {
            values = floatArrayOf(10.0f, 100.0f, 250.0f, 500.0f, 750.0f, 1000.0f, 1000.0f, 1000.0f)
            colors = floatArrayOf(
                0f/255f, 0f/255f, 0f/255f, 0.0f,
                253f/255f, 224f/255f, 71f/255f, 0.3f,
                250f/255f, 204f/255f, 21f/255f, 0.5f,
                234f/255f, 179f/255f, 8f/255f, 0.7f,
                249f/255f, 115f/255f, 22f/255f, 0.85f,
                239f/255f, 68f/255f, 68f/255f, 0.95f,
                239f/255f, 68f/255f, 68f/255f, 0.95f,
                239f/255f, 68f/255f, 68f/255f, 0.95f
            )
        } else if (activeLayerMode == "wind") {
            values = floatArrayOf(0.0f, 2.0f, 5.0f, 10.0f, 15.0f, 20.0f, 25.0f, 25.0f)
            colors = floatArrayOf(
                96f/255f, 165f/255f, 250f/255f, 0.02f,
                34f/255f, 211f/255f, 238f/255f, 0.35f,
                74f/255f, 222f/255f, 128f/255f, 0.55f,
                250f/255f, 204f/255f, 21f/255f, 0.7f,
                251f/255f, 146f/255f, 60f/255f, 0.8f,
                248f/255f, 113f/255f, 113f/255f, 0.85f,
                236f/255f, 72f/255f, 153f/255f, 0.9f,
                236f/255f, 72f/255f, 153f/255f, 0.9f
            )
        } else if (activeEnsemble == "prob") {
            values = floatArrayOf(0.10f, 0.30f, 0.50f, 0.70f, 0.90f, 1.00f, 1.00f, 1.00f)
            colors = floatArrayOf(
                180f/255f, 200f/255f, 220f/255f, 0.0f,
                100f/255f, 160f/255f, 255f/255f, 0.5f,
                0f/255f, 100f/255f, 255f/255f, 0.65f,
                0f/255f, 200f/255f, 100f/255f, 0.75f,
                220f/255f, 0f/255f, 220f/255f, 0.85f,
                255f/255f, 255f/255f, 255f/255f, 0.95f,
                255f/255f, 255f/255f, 255f/255f, 0.95f,
                255f/255f, 255f/255f, 255f/255f, 0.95f
            )
        } else if (activeEnsemble == "spread") {
            values = floatArrayOf(0.05f, 0.2f, 1.0f, 5.0f, 15.0f, 30.0f, 30.0f, 30.0f)
            colors = floatArrayOf(
                99f/255f, 102f/255f, 241f/255f, 0.0f,
                99f/255f, 102f/255f, 241f/255f, 0.4f,
                168f/255f, 85f/255f, 247f/255f, 0.6f,
                236f/255f, 72f/255f, 153f/255f, 0.75f,
                244f/255f, 63f/255f, 94f/255f, 0.9f,
                255f/255f, 255f/255f, 255f/255f, 0.95f,
                255f/255f, 255f/255f, 255f/255f, 0.95f,
                255f/255f, 255f/255f, 255f/255f, 0.95f
            )
        } else {
            values = floatArrayOf(0.05f, 0.2f, 1.0f, 5.0f, 15.0f, 30.0f, 100.0f, 250.0f)
            colors = floatArrayOf(
                120f/255f, 200f/255f, 255f/255f, 0.0f,
                0f/255f, 100f/255f, 255f/255f, 0.7f,
                0f/255f, 200f/255f, 0f/255f, 0.7f,
                255f/255f, 230f/255f, 0f/255f, 0.8f,
                255f/255f, 120f/255f, 0f/255f, 0.9f,
                255f/255f, 0f/255f, 0f/255f, 0.95f,
                200f/255f, 0f/255f, 200f/255f, 1.0f,
                255f/255f, 255f/255f, 255f/255f, 1.0f
            )
        }

        overlay.setStops(values, colors)
    }

    private fun fetchTextures(timeVal: Int, prefetchTimeVals: List<*>) {
        currentRequestSeq++
        val seq = currentRequestSeq
        val activeUrl = buildUrl(timeVal)

        val keepUrls = mutableSetOf<String>()
        if (activeUrl.isNotEmpty()) {
            keepUrls.add(activeUrl)
        }
        for (item in prefetchTimeVals) {
            val tVal = (item as? Number)?.toInt() ?: continue
            val prefetchUrl = buildUrl(tVal)
            if (prefetchUrl.isNotEmpty()) {
                keepUrls.add(prefetchUrl)
            }
        }

        overlay.cancelStaleRequests(keepUrls)

        if (activeUrl.isNotEmpty()) {
            overlay.loadTextureAsync(activeUrl, seq)
        }

        // Prefetch future frames
        for (item in prefetchTimeVals) {
            val tVal = (item as? Number)?.toInt() ?: continue
            val prefetchUrl = buildUrl(tVal)
            if (prefetchUrl.isNotEmpty()) {
                overlay.prefetchTexture(prefetchUrl)
            }
        }
    }


    private fun buildUrl(timeVal: Int): String {
        val relativePath = when (activeLayerMode) {
            "temp" -> "/api/data/temp/$timeVal"
            "solar" -> "/api/data/solar/$timeVal"
            "wind" -> "/api/data/wind/$activeWindHeight/$timeVal"
            else -> "/api/data/$activeEnsemble/$timeVal"
        }

        // Retrieve server base url from shared preferences
        val prefs = context.getSharedPreferences("FlutterSharedPreferences", Context.MODE_PRIVATE)
        val baseUrl = prefs.getString("flutter.nimbus_api_base_url", "") ?: ""
        if (baseUrl.isEmpty()) return ""

        return "$baseUrl$relativePath?v=$activeVersion"
    }

    override fun dispose() {
        mapView.onDestroy()
        glSurfaceView.onPause()
        channel.setMethodCallHandler(null)
    }
}
