import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'app_state.dart';
import 'map_widget.dart';
import 'trend_chart.dart';
import 'api.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  SystemChrome.setSystemUIOverlayStyle(
    const SystemUiOverlayStyle(
      statusBarColor: Colors.transparent,
      statusBarIconBrightness: Brightness.light,
    ),
  );
  runApp(
    ChangeNotifierProvider(
      create: (_) => AppState(),
      child: const NimbusApp(),
    ),
  );
}

class NimbusApp extends StatelessWidget {
  const NimbusApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Nimbus Mobile',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        brightness: Brightness.dark,
        primaryColor: const Color(0xFF2563EB),
        scaffoldBackgroundColor: const Color(0xFF0C0A09),
        fontFamily: 'Inter',
        useMaterial3: true,
      ),
      home: const BootloaderScreen(),
    );
  }
}

class BootloaderScreen extends StatefulWidget {
  const BootloaderScreen({super.key});

  @override
  State<BootloaderScreen> createState() => _BootloaderScreenState();
}

class _BootloaderScreenState extends State<BootloaderScreen> {
  bool _isChecked = false;
  bool _hasUrl = false;

  @override
  void initState() {
    super.initState();
    _checkServerUrl();
  }

  Future<void> _checkServerUrl() async {
    final baseUrl = await NimbusApi.getBaseUrl();
    if (!mounted) return;
    if (baseUrl.isNotEmpty) {
      // Pre-load metadata to check connection
      try {
        final state = Provider.of<AppState>(context, listen: false);
        await state.loadMetadata();
        setState(() {
          _hasUrl = true;
          _isChecked = true;
        });
      } catch (e) {
        // Connection error or stale URL
        setState(() {
          _hasUrl = false;
          _isChecked = true;
        });
      }
    } else {
      setState(() {
        _hasUrl = false;
        _isChecked = true;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    if (!_isChecked) {
      return const Scaffold(
        body: Center(
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Icon(Icons.umbrella, size: 64, color: Color(0xFF2563EB)),
              SizedBox(height: 16),
              CircularProgressIndicator(),
              SizedBox(height: 12),
              Text('Connecting to Nimbus...',
                  style: TextStyle(color: Color(0xFF94A3B8))),
            ],
          ),
        ),
      );
    }

    if (_hasUrl) {
      return const DashboardScreen();
    } else {
      return ConnectionSetupScreen(onConnected: () {
        _checkServerUrl();
      });
    }
  }
}

class ConnectionSetupScreen extends StatefulWidget {
  final VoidCallback onConnected;

  const ConnectionSetupScreen({super.key, required this.onConnected});

  @override
  State<ConnectionSetupScreen> createState() => _ConnectionSetupScreenState();
}

class _ConnectionSetupScreenState extends State<ConnectionSetupScreen> {
  final _controller = TextEditingController();
  bool _isLoading = false;
  String? _errorMessage;

  Future<void> _testAndSave() async {
    setState(() {
      _isLoading = true;
      _errorMessage = null;
    });

    final input = _controller.text.trim();
    if (input.isEmpty) {
      setState(() {
        _isLoading = false;
        _errorMessage = 'Please enter a server URL';
      });
      return;
    }

    // Ensure it starts with http:// or https://
    String targetUrl = input;
    if (!targetUrl.startsWith('http://') && !targetUrl.startsWith('https://')) {
      targetUrl = 'http://$targetUrl';
    }

    final success = await NimbusApi.testConnection(targetUrl);
    if (success) {
      await NimbusApi.setBaseUrl(targetUrl);
      widget.onConnected();
    } else {
      setState(() {
        _isLoading = false;
        _errorMessage =
            'Could not connect. Ensure server is running and accessible.';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: Container(
        decoration: const BoxDecoration(
          gradient: LinearGradient(
            colors: [Color(0xFF0C0A09), Color(0xFF1C1917)],
            begin: Alignment.topCenter,
            end: Alignment.bottomCenter,
          ),
        ),
        child: Center(
          child: SingleChildScrollView(
            child: Padding(
              padding: const EdgeInsets.all(24.0),
              child: Container(
                padding: const EdgeInsets.all(24),
                decoration: BoxDecoration(
                  color: const Color(0xCC1E1E24),
                  borderRadius: BorderRadius.circular(16),
                  border: Border.all(color: const Color(0x1AFFFFFF)),
                  boxShadow: const [
                    BoxShadow(
                      color: Colors.black54,
                      blurRadius: 30,
                      offset: Offset(0, 10),
                    )
                  ],
                ),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    const Icon(Icons.dns, size: 48, color: Color(0xFF38BDF8)),
                    const SizedBox(height: 12),
                    const Text(
                      'Connect to Nimbus Server',
                      style: TextStyle(
                          fontSize: 18,
                          fontWeight: FontWeight.bold,
                          color: Colors.white),
                    ),
                    const SizedBox(height: 8),
                    const Text(
                      'Enter the IP address or URL of your running Nimbus service backend.',
                      textAlign: TextAlign.center,
                      style: TextStyle(fontSize: 12, color: Color(0xFF94A3B8)),
                    ),
                    const SizedBox(height: 20),
                    TextField(
                      controller: _controller,
                      keyboardType: TextInputType.url,
                      decoration: InputDecoration(
                        hintText: 'e.g. http://192.168.1.100:8080',
                        filled: true,
                        fillColor: const Color(0x11FFFFFF),
                        border: OutlineInputBorder(
                          borderRadius: BorderRadius.circular(8),
                          borderSide:
                              const BorderSide(color: Color(0x22FFFFFF)),
                        ),
                        focusedBorder: OutlineInputBorder(
                          borderRadius: BorderRadius.circular(8),
                          borderSide:
                              const BorderSide(color: Color(0xFF38BDF8)),
                        ),
                      ),
                      style: const TextStyle(color: Colors.white),
                    ),
                    if (_errorMessage != null) ...[
                      const SizedBox(height: 12),
                      Row(
                        children: [
                          const Icon(Icons.error_outline,
                              color: Color(0xFFF87171), size: 16),
                          const SizedBox(width: 8),
                          Expanded(
                            child: Text(
                              _errorMessage!,
                              style: const TextStyle(
                                  color: Color(0xFFF87171), fontSize: 11),
                            ),
                          )
                        ],
                      )
                    ],
                    const SizedBox(height: 20),
                    SizedBox(
                      width: double.infinity,
                      child: ElevatedButton.icon(
                        onPressed: _isLoading ? null : _testAndSave,
                        icon: _isLoading
                            ? const SizedBox(
                                width: 16,
                                height: 16,
                                child: CircularProgressIndicator(
                                    strokeWidth: 2, color: Colors.white),
                              )
                            : const Icon(Icons.login),
                        label: Text(_isLoading ? 'Connecting...' : 'Connect'),
                        style: ElevatedButton.styleFrom(
                          backgroundColor: const Color(0xFF2563EB),
                          foregroundColor: Colors.white,
                          padding: const EdgeInsets.symmetric(vertical: 14),
                          shape: RoundedRectangleBorder(
                            borderRadius: BorderRadius.circular(8),
                          ),
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class DashboardScreen extends StatefulWidget {
  const DashboardScreen({super.key});

  @override
  State<DashboardScreen> createState() => _DashboardScreenState();
}

class _DashboardScreenState extends State<DashboardScreen> {
  bool _isSettingsExpanded = false;

  String _formatTimeLabel(AppState state) {
    if (state.activeMetadata == null || state.activeMetadata!.times.isEmpty) {
      return 'Loading...';
    }
    try {
      final secs = state.activeMetadata!.times[state.currentTimeIndex];
      final match = RegExp(r"(\d{4}-\d{2}-\d{2})\s+(\d{2}:\d{2}:\d{2})")
          .firstMatch(state.activeMetadata!.referenceTimeStr);
      if (match != null) {
        final refTime =
            DateTime.parse("${match.group(1)}T${match.group(2)}Z").toUtc();
        final stepTime = refTime.add(Duration(seconds: secs)).toLocal();

        final hour = stepTime.hour.toString().padLeft(2, '0');
        final min = stepTime.minute.toString().padLeft(2, '0');
        final day = stepTime.day.toString().padLeft(2, '0');
        final month = _getMonthName(stepTime.month);

        return '$day $month $hour:$min';
      }
    } catch (_) {}
    return 'Step ${state.currentTimeIndex}';
  }

  String _getMonthName(int month) {
    const months = [
      'Jan',
      'Feb',
      'Mar',
      'Apr',
      'May',
      'Jun',
      'Jul',
      'Aug',
      'Sep',
      'Oct',
      'Nov',
      'Dec'
    ];
    return months[month - 1];
  }

  String _formatReferenceTime(AppState state) {
    if (state.activeMetadata == null) return '';
    try {
      final match = RegExp(r"(\d{4}-\d{2}-\d{2})\s+(\d{2}:\d{2}:\d{2})")
          .firstMatch(state.activeMetadata!.referenceTimeStr);
      if (match != null) {
        final refTime =
            DateTime.parse("${match.group(1)}T${match.group(2)}Z").toLocal();
        final hour = refTime.hour.toString().padLeft(2, '0');
        final min = refTime.minute.toString().padLeft(2, '0');
        return '$hour:$min';
      }
    } catch (_) {}
    return state.activeMetadata!.referenceTimeStr;
  }

  @override
  Widget build(BuildContext context) {
    final appState = Provider.of<AppState>(context);

    return Scaffold(
      body: Stack(
        children: [
          // Native Platform Map View
          NimbusMapWidget(
            onMapClick: (lat, lon) {
              appState.setSelectedLocation(lat, lon);
            },
          ),

          // Hover panel or selected point value
          _buildHoverValuePanel(appState),

          // Trend chart widget on right side
          if (appState.activeLat != null)
            TrendChartWidget(
              appState: appState,
              onClose: () {
                appState.clearSelectedLocation();
              },
            ),

          // Controls Dashboard (Bottom)
          Align(
            alignment: Alignment.bottomCenter,
            child: SafeArea(
              child: Container(
                margin: const EdgeInsets.all(16),
                width: double.infinity,
                constraints: const BoxConstraints(maxWidth: 640),
                decoration: BoxDecoration(
                  color: const Color(0xCC121216),
                  borderRadius: BorderRadius.circular(16),
                  border:
                      Border.all(color: const Color(0x17FFFFFF), width: 1.0),
                  boxShadow: const [
                    BoxShadow(
                      color: Colors.black45,
                      blurRadius: 20,
                      offset: Offset(0, 4),
                    )
                  ],
                ),
                child: Padding(
                  padding: const EdgeInsets.all(16.0),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      // Header Row (Logo, Settings, Info)
                      Row(
                        mainAxisAlignment: MainAxisAlignment.spaceBetween,
                        children: [
                          const Row(
                            children: [
                              Icon(Icons.umbrella,
                                  color: Color(0xFF3B82F6), size: 20),
                              SizedBox(width: 8),
                              Text(
                                'Nimbus',
                                style: TextStyle(
                                    fontWeight: FontWeight.bold,
                                    fontSize: 16,
                                    color: Colors.white),
                              ),
                              SizedBox(width: 4),
                              Card(
                                color: Color(0xFF2563EB),
                                child: Padding(
                                  padding: EdgeInsets.symmetric(
                                      horizontal: 6, vertical: 2),
                                  child: Text('NATIVE',
                                      style: TextStyle(
                                          fontSize: 8,
                                          fontWeight: FontWeight.bold)),
                                ),
                              )
                            ],
                          ),
                          Row(
                            children: [
                              if (appState.activeMetadata != null)
                                Text(
                                  'Ref: ${_formatReferenceTime(appState)}',
                                  style: const TextStyle(
                                      color: Color(0xFF94A3B8), fontSize: 11),
                                ),
                              const SizedBox(width: 12),
                              IconButton(
                                constraints: const BoxConstraints(),
                                padding: EdgeInsets.zero,
                                icon: Icon(
                                  Icons.tune,
                                  color: _isSettingsExpanded
                                      ? const Color(0xFF2563EB)
                                      : const Color(0xFF94A3B8),
                                  size: 20,
                                ),
                                onPressed: () {
                                  setState(() {
                                    _isSettingsExpanded = !_isSettingsExpanded;
                                  });
                                },
                              ),
                            ],
                          )
                        ],
                      ),
                      const Divider(color: Color(0x17FFFFFF), height: 16),

                      // Time step and relative offset
                      Row(
                        mainAxisAlignment: MainAxisAlignment.spaceBetween,
                        children: [
                          Text(
                            _formatTimeLabel(appState),
                            style: const TextStyle(
                                fontWeight: FontWeight.w600,
                                fontSize: 14,
                                color: Colors.white),
                          ),
                          if (appState.activeMetadata != null)
                            Text(
                              '+${appState.activeMetadata!.times[appState.currentTimeIndex] ~/ 60}m',
                              style: const TextStyle(
                                  color: Color(0xFF94A3B8), fontSize: 11),
                            )
                        ],
                      ),
                      const SizedBox(height: 8),

                      // Slider
                      if (appState.activeMetadata != null)
                        SliderTheme(
                          data: SliderTheme.of(context).copyWith(
                            trackHeight: 3.0,
                            thumbShape: const RoundSliderThumbShape(
                                enabledThumbRadius: 7.0),
                            overlayShape: const RoundSliderOverlayShape(
                                overlayRadius: 14.0),
                          ),
                          child: Slider(
                            value: appState.currentTimeIndex.toDouble(),
                            min: 0,
                            max: (appState.activeMetadata!.times.length - 1)
                                .toDouble(),
                            activeColor: const Color(0xFF2563EB),
                            inactiveColor: const Color(0x22FFFFFF),
                            onChanged: (val) {
                              appState.setTimeIndex(val.toInt());
                            },
                          ),
                        ),
                      const SizedBox(height: 8),

                      // Player Row (Layers, Stats, Play buttons)
                      Row(
                        mainAxisAlignment: MainAxisAlignment.spaceBetween,
                        children: [
                          // Mode selector (Rain, Temp, Wind, Solar)
                          _buildModeSelector(appState),

                          // Play/Pause buttons
                          Row(
                            children: [
                              GestureDetector(
                                onTap: () {
                                  final next = (appState.currentTimeIndex -
                                          1 +
                                          (appState.activeMetadata?.times
                                                  .length ??
                                              1)) %
                                      (appState.activeMetadata?.times.length ??
                                          1);
                                  appState.setTimeIndex(next);
                                },
                                child: const Padding(
                                  padding: EdgeInsets.symmetric(
                                      horizontal: 6.0, vertical: 8.0),
                                  child: Icon(Icons.arrow_left,
                                      size: 24, color: Colors.white),
                                ),
                              ),
                              const SizedBox(width: 4),
                              GestureDetector(
                                onTap: () => appState.togglePlayback(),
                                child: Container(
                                  width: 34,
                                  height: 34,
                                  decoration: const BoxDecoration(
                                    color: Color(0xFF2563EB),
                                    shape: BoxShape.circle,
                                    boxShadow: [
                                      BoxShadow(
                                          color: Color(0x662563EB),
                                          blurRadius: 8)
                                    ],
                                  ),
                                  child: Icon(
                                    appState.isPlaying
                                        ? Icons.pause
                                        : Icons.play_arrow,
                                    color: Colors.white,
                                    size: 18,
                                  ),
                                ),
                              ),
                              const SizedBox(width: 4),
                              GestureDetector(
                                onTap: () {
                                  final next = (appState.currentTimeIndex + 1) %
                                      (appState.activeMetadata?.times.length ??
                                          1);
                                  appState.setTimeIndex(next);
                                },
                                child: const Padding(
                                  padding: EdgeInsets.symmetric(
                                      horizontal: 6.0, vertical: 8.0),
                                  child: Icon(Icons.arrow_right,
                                      size: 24, color: Colors.white),
                                ),
                              ),
                            ],
                          )
                        ],
                      ),

                      // Collapsible Settings
                      AnimatedCrossFade(
                        firstChild: _buildSettingsDrawer(appState),
                        secondChild: const SizedBox.shrink(),
                        crossFadeState: _isSettingsExpanded
                            ? CrossFadeState.showFirst
                            : CrossFadeState.showSecond,
                        duration: const Duration(milliseconds: 200),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildHoverValuePanel(AppState state) {
    if (state.activeLat == null) return const SizedBox.shrink();
    return Positioned(
      top: 24,
      left: 0,
      right: 0,
      child: Center(
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
          decoration: BoxDecoration(
            color: const Color(0x99121216),
            borderRadius: BorderRadius.circular(12),
            border: Border.all(color: const Color(0x17FFFFFF)),
          ),
          child: const Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Text('SELECTED LOCATION',
                  style: TextStyle(
                      color: Color(0xFF94A3B8),
                      fontSize: 9,
                      fontWeight: FontWeight.bold)),
              SizedBox(height: 2),
              Text(
                'Tapped Location',
                style: TextStyle(
                    color: Colors.white,
                    fontSize: 13,
                    fontWeight: FontWeight.bold),
              )
            ],
          ),
        ),
      ),
    );
  }

  Widget _buildModeSelector(AppState state) {
    final modes = ['rain', 'temp', 'wind', 'solar'];
    final icons = {
      'rain': Icons.umbrella,
      'temp': Icons.thermostat,
      'wind': Icons.air,
      'solar': Icons.wb_sunny,
    };

    return Row(
      children: modes.map((m) {
        final active = state.currentLayerMode == m;
        return GestureDetector(
          onTap: () => state.setLayerMode(m),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 6.0, vertical: 8.0),
            child: Icon(
              icons[m],
              color: active ? const Color(0xFF2563EB) : const Color(0xFF94A3B8),
              size: 20,
            ),
          ),
        );
      }).toList(),
    );
  }

  Widget _buildSettingsDrawer(AppState state) {
    return Column(
      children: [
        const Divider(color: Color(0x17FFFFFF), height: 16),
        // Opacity
        Row(
          children: [
            const Icon(Icons.opacity, color: Color(0xFF94A3B8), size: 16),
            const SizedBox(width: 8),
            const Text('Opacity',
                style: TextStyle(color: Color(0xFF94A3B8), fontSize: 12)),
            Expanded(
              child: Slider(
                value: state.opacity,
                min: 0.0,
                max: 1.0,
                onChanged: (val) => state.setOpacity(val),
              ),
            ),
            Text('${(state.opacity * 100).round()}%',
                style: const TextStyle(fontSize: 11)),
          ],
        ),
        // Speed
        Row(
          children: [
            const Icon(Icons.speed, color: Color(0xFF94A3B8), size: 16),
            const SizedBox(width: 8),
            const Text('Speed',
                style: TextStyle(color: Color(0xFF94A3B8), fontSize: 12)),
            Expanded(
              child: Slider(
                value: state.speedFps.toDouble(),
                min: 1.0,
                max: 10.0,
                divisions: 9,
                onChanged: (val) => state.setSpeedFps(val.toInt()),
              ),
            ),
            Text('${state.speedFps} fps', style: const TextStyle(fontSize: 11)),
          ],
        ),
        const SizedBox(height: 8),
        // Server Info & Reset
        FutureBuilder<String>(
          future: NimbusApi.getBaseUrl(),
          builder: (context, snapshot) {
            final url = snapshot.data ?? '';
            return Container(
              padding: const EdgeInsets.all(8),
              decoration: BoxDecoration(
                color: const Color(0x0EFFFFFF),
                borderRadius: BorderRadius.circular(8),
              ),
              child: Row(
                mainAxisAlignment: MainAxisAlignment.spaceBetween,
                children: [
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        const Text('SERVER',
                            style: TextStyle(
                                color: Color(0xFF94A3B8),
                                fontSize: 8,
                                fontWeight: FontWeight.bold)),
                        Text(
                          url,
                          style: const TextStyle(
                              color: Colors.white, fontSize: 11),
                          overflow: TextOverflow.ellipsis,
                        ),
                      ],
                    ),
                  ),
                  TextButton.icon(
                    onPressed: () async {
                      final prefs = await SharedPreferences.getInstance();
                      await prefs.remove('nimbus_api_base_url');
                      if (context.mounted) {
                        Navigator.of(context).pushAndRemoveUntil(
                          MaterialPageRoute(
                              builder: (_) => const BootloaderScreen()),
                          (route) => false,
                        );
                      }
                    },
                    icon: const Icon(Icons.link_off,
                        size: 14, color: Color(0xFFF87171)),
                    label: const Text('Disconnect',
                        style:
                            TextStyle(color: Color(0xFFF87171), fontSize: 11)),
                  )
                ],
              ),
            );
          },
        ),
      ],
    );
  }
}
