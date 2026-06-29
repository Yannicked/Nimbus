import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import 'app_state.dart';

typedef MapClickCallback = void Function(double lat, double lon);

class NimbusMapWidget extends StatefulWidget {
  final MapClickCallback onMapClick;

  const NimbusMapWidget({super.key, required this.onMapClick});

  @override
  State<NimbusMapWidget> createState() => _NimbusMapWidgetState();
}

class _NimbusMapWidgetState extends State<NimbusMapWidget> {
  MethodChannel? _channel;
  late AppState _appState;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _appState = Provider.of<AppState>(context);
    _syncStateToNative();
  }

  void _onPlatformViewCreated(int id) {
    _channel = MethodChannel('com.yannicked.nimbus/map_control_$id');
    _channel!.setMethodCallHandler(_handleNativeMethodCall);
    _syncStateToNative();
  }

  Future<void> _handleNativeMethodCall(MethodCall call) async {
    switch (call.method) {
      case 'onMapClick':
        final Map<dynamic, dynamic> args = call.arguments;
        final double lat = args['lat'];
        final double lon = args['lon'];
        widget.onMapClick(lat, lon);
        break;
      default:
        debugPrint('Unknown native method call: ${call.method}');
    }
  }

  void _syncStateToNative() {
    if (_channel == null || _appState.activeMetadata == null) return;

    final data = {
      'layerMode': _appState.currentLayerMode,
      'ensemble': _appState.currentEns,
      'windHeight': _appState.selectedWindHeight,
      'currentTimeIndex': _appState.currentTimeIndex,
      'opacity': _appState.opacity,
      'timeVal': _appState.activeMetadata!.times[_appState.currentTimeIndex],
      'version': _appState.activeMetadata!.version.toString(),
      'selectedLat': _appState.activeLat,
      'selectedLon': _appState.activeLon,
      'prefetchTimeVals': _appState.activeMetadata!.times
          .skip(_appState.currentTimeIndex + 1)
          .take(5)
          .toList(),
    };

    _channel!.invokeMethod('syncState', data).catchError((e) {
      debugPrint('Error syncing state to native map: $e');
    });
  }

  @override
  Widget build(BuildContext context) {
    const String viewType = 'com.yannicked.nimbus/map_view';
    final Map<String, dynamic> creationParams = {
      'layerMode': _appState.currentLayerMode,
      'ensemble': _appState.currentEns,
      'windHeight': _appState.selectedWindHeight,
      'currentTimeIndex': _appState.currentTimeIndex,
      'opacity': _appState.opacity,
      'selectedLat': _appState.activeLat,
      'selectedLon': _appState.activeLon,
    };

    if (defaultTargetPlatform == TargetPlatform.android) {
      return AndroidView(
        viewType: viewType,
        onPlatformViewCreated: _onPlatformViewCreated,
        creationParams: creationParams,
        creationParamsCodec: const StandardMessageCodec(),
      );
    } else if (defaultTargetPlatform == TargetPlatform.iOS) {
      return UiKitView(
        viewType: viewType,
        onPlatformViewCreated: _onPlatformViewCreated,
        creationParams: creationParams,
        creationParamsCodec: const StandardMessageCodec(),
      );
    }

    return const Center(
      child: Text('Map View not supported on this platform'),
    );
  }
}
