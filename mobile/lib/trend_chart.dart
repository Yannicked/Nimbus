import 'package:flutter/material.dart';
import 'package:fl_chart/fl_chart.dart';
import 'app_state.dart';
import 'models.dart';
import 'api.dart';

class TrendChartWidget extends StatefulWidget {
  final AppState appState;
  final VoidCallback onClose;

  const TrendChartWidget(
      {super.key, required this.appState, required this.onClose});

  @override
  State<TrendChartWidget> createState() => _TrendChartWidgetState();
}

class _TrendChartWidgetState extends State<TrendChartWidget> {
  Future<TimeseriesResult>? _timeseriesFuture;
  Future<WindTimeseriesResult>? _windTimeseriesFuture;

  @override
  void initState() {
    super.initState();
    _loadData();
  }

  @override
  void didUpdateWidget(covariant TrendChartWidget oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.appState.activeLat != widget.appState.activeLat ||
        oldWidget.appState.activeLon != widget.appState.activeLon ||
        oldWidget.appState.currentLayerMode !=
            widget.appState.currentLayerMode ||
        oldWidget.appState.currentEns != widget.appState.currentEns ||
        oldWidget.appState.selectedWindHeight !=
            widget.appState.selectedWindHeight) {
      _loadData();
    }
  }

  void _loadData() {
    final lat = widget.appState.activeLat;
    final lon = widget.appState.activeLon;
    if (lat == null || lon == null) return;

    if (widget.appState.currentLayerMode == 'wind') {
      _windTimeseriesFuture = NimbusApi.fetchWindTimeseries(
          lat, lon, widget.appState.selectedWindHeight);
      _timeseriesFuture = null;
    } else {
      _timeseriesFuture = NimbusApi.fetchTimeseries(
        widget.appState.currentLayerMode,
        widget.appState.currentEns,
        lat,
        lon,
        widget.appState.selectedWindHeight,
      );
      _windTimeseriesFuture = null;
    }
    setState(() {});
  }

  String _formatRelativeTime(int seconds) {
    final minutes = seconds ~/ 60;
    if (minutes < 60) {
      return '+${minutes}m';
    }
    final hours = minutes / 60;
    return '+${hours.toStringAsFixed(1)}h';
  }

  Color _getThemeColor() {
    final mode = widget.appState.currentLayerMode;
    final ens = widget.appState.currentEns;
    if (mode == 'temp') return const Color(0xFFF87171); // Light red
    if (mode == 'solar') return const Color(0xFFF59E0B); // Amber
    if (mode == 'wind') return const Color(0xFF22D3EE); // Cyan
    if (ens == 'prob') return const Color(0xFFA855F7); // Purple
    if (ens == 'spread') return const Color(0xFFEC4899); // Pink
    return const Color(0xFF38BDF8); // Sky blue
  }

  @override
  Widget build(BuildContext context) {
    final lat = widget.appState.activeLat;
    final lon = widget.appState.activeLon;
    if (lat == null || lon == null) return const SizedBox.shrink();

    final themeColor = _getThemeColor();

    return Align(
      alignment: Alignment.topRight,
      child: Container(
        margin: const EdgeInsets.only(top: 80, right: 16),
        width: 320,
        height: 290,
        decoration: BoxDecoration(
          color: const Color(0x99121216),
          borderRadius: BorderRadius.circular(16),
          border: Border.all(color: const Color(0x17FFFFFF), width: 1.0),
          boxShadow: const [
            BoxShadow(
              color: Color(0x99000000),
              blurRadius: 24,
              offset: Offset(0, 8),
            )
          ],
        ),
        child: ClipRRect(
          borderRadius: BorderRadius.circular(16),
          child: Padding(
            padding: const EdgeInsets.all(12.0),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                // Header
                Row(
                  mainAxisAlignment: MainAxisAlignment.spaceBetween,
                  children: [
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(
                            _getHeaderTitle(),
                            style: const TextStyle(
                              color: Colors.white,
                              fontSize: 13,
                              fontWeight: FontWeight.bold,
                            ),
                          ),
                          const SizedBox(height: 2),
                          Text(
                            'lat: ${lat.toStringAsFixed(4)}, lon: ${lon.toStringAsFixed(4)}',
                            style: const TextStyle(
                              color: Color(0xFF94A3B8),
                              fontSize: 10,
                            ),
                          ),
                        ],
                      ),
                    ),
                    IconButton(
                      constraints: const BoxConstraints(),
                      padding: EdgeInsets.zero,
                      icon: const Icon(Icons.close,
                          color: Color(0xFF94A3B8), size: 18),
                      onPressed: widget.onClose,
                    )
                  ],
                ),
                const SizedBox(height: 10),

                // Chart Container
                Expanded(
                  child: widget.appState.currentLayerMode == 'wind'
                      ? _buildWindChart(themeColor)
                      : _buildStandardChart(themeColor),
                ),
                const SizedBox(height: 10),

                // Stats Section
                if (widget.appState.currentLayerMode == 'wind')
                  _buildWindStats()
                else
                  _buildStandardStats(),
              ],
            ),
          ),
        ),
      ),
    );
  }

  String _getHeaderTitle() {
    final mode = widget.appState.currentLayerMode;
    final ens = widget.appState.currentEns;
    if (mode == 'temp') return 'Temperature Forecast Trend';
    if (mode == 'solar') return 'Solar Forecast Trend';
    if (mode == 'wind') return 'Wind Speed Forecast Trend';
    if (ens == 'spread') return 'Forecast Uncertainty Trend';
    return 'Rainfall Forecast Trend';
  }

  Widget _buildStandardChart(Color themeColor) {
    return FutureBuilder<TimeseriesResult>(
      future: _timeseriesFuture,
      builder: (context, snapshot) {
        if (snapshot.connectionState == ConnectionState.waiting) {
          return const Center(child: CircularProgressIndicator());
        }
        if (snapshot.hasError || !snapshot.hasData) {
          return const Center(
            child: Text('Error loading chart data',
                style: TextStyle(color: Colors.red, fontSize: 10)),
          );
        }

        final data = snapshot.data!;
        if (data.status == 'out_of_bounds' || data.values.isEmpty) {
          return const Center(
            child: Text('Location is out of bounds',
                style: TextStyle(color: Color(0xFF94A3B8), fontSize: 11)),
          );
        }

        final spots = <FlSpot>[];
        for (int i = 0; i < data.values.length; i++) {
          spots.add(FlSpot(i.toDouble(), data.values[i]));
        }

        return _buildFlChart(spots, data.times, themeColor);
      },
    );
  }

  Widget _buildWindChart(Color themeColor) {
    return FutureBuilder<WindTimeseriesResult>(
      future: _windTimeseriesFuture,
      builder: (context, snapshot) {
        if (snapshot.connectionState == ConnectionState.waiting) {
          return const Center(child: CircularProgressIndicator());
        }
        if (snapshot.hasError || !snapshot.hasData) {
          return const Center(
            child: Text('Error loading chart data',
                style: TextStyle(color: Colors.red, fontSize: 10)),
          );
        }

        final data = snapshot.data!;
        if (data.status == 'out_of_bounds' || data.speeds.isEmpty) {
          return const Center(
            child: Text('Location is out of bounds',
                style: TextStyle(color: Color(0xFF94A3B8), fontSize: 11)),
          );
        }

        final spots = <FlSpot>[];
        for (int i = 0; i < data.speeds.length; i++) {
          spots.add(FlSpot(i.toDouble(), data.speeds[i]));
        }

        return _buildFlChart(spots, data.times, themeColor);
      },
    );
  }

  Widget _buildFlChart(List<FlSpot> spots, List<int> times, Color themeColor) {
    return LineChart(
      LineChartData(
        gridData: FlGridData(
          show: true,
          drawVerticalLine: true,
          horizontalInterval: _getYInterval(),
          verticalInterval: (spots.length / 5).clamp(1.0, 100.0),
          getDrawingHorizontalLine: (value) =>
              const FlLine(color: Color(0x0EFFFFFF), strokeWidth: 1),
          getDrawingVerticalLine: (value) =>
              const FlLine(color: Color(0x0EFFFFFF), strokeWidth: 1),
        ),
        titlesData: FlTitlesData(
          show: true,
          rightTitles:
              const AxisTitles(sideTitles: SideTitles(showTitles: false)),
          topTitles:
              const AxisTitles(sideTitles: SideTitles(showTitles: false)),
          leftTitles: AxisTitles(
            sideTitles: SideTitles(
              showTitles: true,
              reservedSize: 32,
              getTitlesWidget: (value, meta) {
                return Text(
                  value.toStringAsFixed(value % 1 == 0 ? 0 : 1),
                  style: const TextStyle(color: Color(0xFF94A3B8), fontSize: 8),
                );
              },
            ),
          ),
          bottomTitles: AxisTitles(
            sideTitles: SideTitles(
              showTitles: true,
              reservedSize: 18,
              getTitlesWidget: (value, meta) {
                final idx = value.toInt();
                if (idx < 0 ||
                    idx >= times.length ||
                    idx % (times.length ~/ 3) != 0) {
                  return const SizedBox.shrink();
                }
                return Padding(
                  padding: const EdgeInsets.only(top: 4.0),
                  child: Text(
                    _formatRelativeTime(times[idx]),
                    style:
                        const TextStyle(color: Color(0xFF94A3B8), fontSize: 8),
                  ),
                );
              },
            ),
          ),
        ),
        borderData: FlBorderData(show: false),
        minX: 0,
        maxX: (spots.length - 1).toDouble(),
        minY: _getMinY(),
        maxY: _getMaxY(spots),
        lineBarsData: [
          LineChartBarData(
            spots: spots,
            isCurved: true,
            color: themeColor,
            barWidth: 2,
            isStrokeCapRound: true,
            dotData: const FlDotData(show: false),
            belowBarData: BarAreaData(
              show: true,
              color: themeColor.withValues(alpha: 0.12),
            ),
          ),
        ],
      ),
    );
  }

  double _getMinY() {
    if (widget.appState.currentLayerMode == 'temp') {
      return -15.0; // Fixed bounds for temp range
    }
    return 0.0;
  }

  double _getMaxY(List<FlSpot> spots) {
    if (spots.isEmpty) return 10.0;
    double max = 0.0;
    for (var spot in spots) {
      if (spot.y > max) max = spot.y;
    }
    if (widget.appState.currentLayerMode == 'temp') {
      return (max + 5.0).clamp(10.0, 45.0);
    }
    if (widget.appState.currentEns == 'prob') {
      return 100.0;
    }
    return max > 0 ? max * 1.25 : 10.0;
  }

  double _getYInterval() {
    if (widget.appState.currentLayerMode == 'temp') return 5.0;
    if (widget.appState.currentEns == 'prob') return 20.0;
    return 10.0;
  }

  Widget _buildStandardStats() {
    return FutureBuilder<TimeseriesResult>(
      future: _timeseriesFuture,
      builder: (context, snapshot) {
        if (!snapshot.hasData || snapshot.data!.values.isEmpty) {
          return const SizedBox.shrink();
        }
        final values = snapshot.data!.values;
        final mode = widget.appState.currentLayerMode;
        final ens = widget.appState.currentEns;

        double peak = 0.0;

        for (var v in values) {
          if (v > peak) peak = v;
        }

        String label1 = 'Peak';
        String label2 = 'Average';
        String val1 = '';
        String val2 = '';

        if (mode == 'temp') {
          double min = values.first;
          for (var v in values) {
            if (v < min) min = v;
          }
          label1 = 'Max Temp';
          label2 = 'Min Temp';
          val1 = '${peak.toStringAsFixed(1)} °C';
          val2 = '${min.toStringAsFixed(1)} °C';
        } else if (mode == 'solar') {
          label1 = 'Peak Solar';
          label2 = 'Avg Solar';
          double sum = values.reduce((a, b) => a + b);
          val1 = '${peak.round()} W/m²';
          val2 = '${(sum / values.length).round()} W/m²';
        } else if (ens == 'prob') {
          label1 = 'Peak Chance';
          label2 = 'Avg Chance';
          double sum = values.reduce((a, b) => a + b);
          val1 = '${peak.round()}%';
          val2 = '${(sum / values.length).round()}%';
        } else if (ens == 'spread') {
          label1 = 'Max Uncertainty';
          label2 = 'Avg Uncertainty';
          double sum = values.reduce((a, b) => a + b);
          val1 = '${peak.toStringAsFixed(2)} mm/h';
          val2 = '${(sum / values.length).toStringAsFixed(2)} mm/h';
        } else {
          // total rain = sum(rates) / 12.0 (rates are mm/h, steps are 5 mins)
          double sum = values.reduce((a, b) => a + b);
          label1 = 'Peak Intensity';
          label2 = 'Total Rain';
          val1 = '${peak.toStringAsFixed(2)} mm/h';
          val2 = '${(sum / 12.0).toStringAsFixed(2)} mm';
        }

        return _renderStatsRow(label1, val1, label2, val2);
      },
    );
  }

  Widget _buildWindStats() {
    return FutureBuilder<WindTimeseriesResult>(
      future: _windTimeseriesFuture,
      builder: (context, snapshot) {
        if (!snapshot.hasData || snapshot.data!.speeds.isEmpty) {
          return const SizedBox.shrink();
        }
        final speeds = snapshot.data!.speeds;

        double max = 0.0;
        double sum = 0.0;

        for (var s in speeds) {
          if (s > max) max = s;
          sum += s;
        }

        return _renderStatsRow(
          'Max Wind',
          '${max.toStringAsFixed(1)} m/s',
          'Avg Wind',
          '${(sum / speeds.length).toStringAsFixed(1)} m/s',
        );
      },
    );
  }

  Widget _renderStatsRow(
      String label1, String val1, String label2, String val2) {
    return Row(
      children: [
        Expanded(child: _renderStatBox(label1, val1)),
        const SizedBox(width: 8),
        Expanded(child: _renderStatBox(label2, val2)),
      ],
    );
  }

  Widget _renderStatBox(String label, String value) {
    return Container(
      padding: const EdgeInsets.symmetric(vertical: 6, horizontal: 8),
      decoration: BoxDecoration(
        color: const Color(0x0FFFFFFF),
        borderRadius: BorderRadius.circular(8),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            label.toUpperCase(),
            style: const TextStyle(
                color: Color(0xFF94A3B8),
                fontSize: 7,
                fontWeight: FontWeight.bold),
          ),
          const SizedBox(height: 2),
          Text(
            value,
            style: const TextStyle(
                color: Colors.white, fontSize: 12, fontWeight: FontWeight.bold),
          ),
        ],
      ),
    );
  }
}
