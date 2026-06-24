import 'dart:async';
import 'package:flutter/foundation.dart';
import 'models.dart';
import 'api.dart';

class AppState extends ChangeNotifier {
  String _currentLayerMode = 'rain'; // rain, temp, wind, solar
  String _currentEns = 'med'; // med, max, prob, spread, or member number
  int _selectedWindHeight = 10; // 10, 50, 100, 200, 300
  int _currentTimeIndex = 0;
  bool _isPlaying = false;
  double _opacity = 0.70;
  int _speedFps = 3;

  ForecastMetadata? rainMetadata;
  ForecastMetadata? tempMetadata;
  ForecastMetadata? windMetadata;
  ForecastMetadata? solarMetadata;

  ForecastMetadata? activeMetadata;
  Timer? _playbackTimer;

  // Selected Location Coordinates
  double? activeLat;
  double? activeLon;

  // Getters
  String get currentLayerMode => _currentLayerMode;
  String get currentEns => _currentEns;
  int get selectedWindHeight => _selectedWindHeight;
  int get currentTimeIndex => _currentTimeIndex;
  bool get isPlaying => _isPlaying;
  double get opacity => _opacity;
  int get speedFps => _speedFps;

  void setLayerMode(String mode) {
    if (_currentLayerMode == mode) return;
    _currentLayerMode = mode;
    _updateActiveMetadata();
    notifyListeners();
  }

  void setEnsemble(String ens) {
    if (_currentEns == ens) return;
    _currentEns = ens;
    notifyListeners();
  }

  void setWindHeight(int height) {
    if (_selectedWindHeight == height) return;
    _selectedWindHeight = height;
    notifyListeners();
  }

  void setTimeIndex(int index) {
    if (activeMetadata == null) return;
    final maxIndex = activeMetadata!.times.length - 1;
    _currentTimeIndex = index.clamp(0, maxIndex);
    notifyListeners();
  }

  void setOpacity(double value) {
    if (_opacity == value) return;
    _opacity = value.clamp(0.0, 1.0);
    notifyListeners();
  }

  void setSpeedFps(int value) {
    if (_speedFps == value) return;
    _speedFps = value.clamp(1, 10);
    if (_isPlaying) {
      _restartPlaybackTimer();
    }
    notifyListeners();
  }

  void setSelectedLocation(double lat, double lon) {
    activeLat = lat;
    activeLon = lon;
    notifyListeners();
  }

  void clearSelectedLocation() {
    activeLat = null;
    activeLon = null;
    notifyListeners();
  }

  void togglePlayback() {
    _isPlaying = !_isPlaying;
    if (_isPlaying) {
      _startPlaybackTimer();
    } else {
      _stopPlaybackTimer();
    }
    notifyListeners();
  }

  Future<void> loadMetadata() async {
    try {
      rainMetadata = await NimbusApi.fetchMetadata('rain');
      tempMetadata = await NimbusApi.fetchMetadata('temp');
      windMetadata = await NimbusApi.fetchMetadata('wind');
      solarMetadata = await NimbusApi.fetchMetadata('solar');
      _updateActiveMetadata();
      _initializeTimeIndex();
      notifyListeners();
    } catch (e) {
      debugPrint("Failed to load metadata in AppState: $e");
      rethrow;
    }
  }

  void _updateActiveMetadata() {
    if (_currentLayerMode == 'temp') {
      activeMetadata = tempMetadata;
    } else if (_currentLayerMode == 'wind') {
      activeMetadata = windMetadata;
    } else if (_currentLayerMode == 'solar') {
      activeMetadata = solarMetadata;
    } else {
      activeMetadata = rainMetadata;
    }
  }

  void _initializeTimeIndex() {
    if (activeMetadata == null || activeMetadata!.times.isEmpty) return;

    // Find step closest to system time
    try {
      final match = RegExp(r"(\d{4}-\d{2}-\d{2})\s+(\d{2}:\d{2}:\d{2})")
          .firstMatch(activeMetadata!.referenceTimeStr);
      if (match != null) {
        final refTime =
            DateTime.parse("${match.group(1)}T${match.group(2)}Z").toUtc();
        final nowUtc = DateTime.now().toUtc();
        final targetOffset = nowUtc.difference(refTime).inSeconds;

        int closestIndex = 0;
        int minDiff = 100000000;
        for (int i = 0; i < activeMetadata!.times.length; i++) {
          final diff = (activeMetadata!.times[i] - targetOffset).abs();
          if (diff < minDiff) {
            minDiff = diff;
            closestIndex = i;
          }
        }
        _currentTimeIndex = closestIndex;
      }
    } catch (_) {
      _currentTimeIndex = 0;
    }
  }

  void _startPlaybackTimer() {
    final intervalMs = (1000 / _speedFps).round();
    _playbackTimer =
        Timer.periodic(Duration(milliseconds: intervalMs), (timer) {
      if (activeMetadata == null || activeMetadata!.times.isEmpty) return;
      final nextIndex = (_currentTimeIndex + 1) % activeMetadata!.times.length;
      setTimeIndex(nextIndex);
    });
  }

  void _stopPlaybackTimer() {
    _playbackTimer?.cancel();
    _playbackTimer = null;
  }

  void _restartPlaybackTimer() {
    _stopPlaybackTimer();
    _startPlaybackTimer();
  }

  @override
  void dispose() {
    _stopPlaybackTimer();
    super.dispose();
  }
}
