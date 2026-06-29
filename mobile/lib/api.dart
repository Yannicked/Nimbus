import 'dart:convert';
import 'dart:typed_data';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';
import 'models.dart';

class NimbusApi {
  static Future<String> getBaseUrl() async {
    final prefs = await SharedPreferences.getInstance();
    return prefs.getString('nimbus_api_base_url') ?? '';
  }

  static Future<void> setBaseUrl(String url) async {
    final prefs = await SharedPreferences.getInstance();
    // Normalize url
    String normalized = url.trim();
    if (normalized.endsWith('/')) {
      normalized = normalized.substring(0, normalized.length - 1);
    }
    await prefs.setString('nimbus_api_base_url', normalized);
  }

  static Future<String> resolveUrl(String relativePath) async {
    final base = await getBaseUrl();
    final cleanPath =
        relativePath.startsWith('/') ? relativePath : '/$relativePath';
    return '$base$cleanPath';
  }

  static Future<ForecastMetadata> fetchMetadata(String layerMode) async {
    final Map<String, String> endpoints = {
      'temp': '/api/metadata/temp',
      'wind': '/api/metadata/wind',
      'solar': '/api/metadata/solar',
      'rain': '/api/metadata',
    };
    final path = endpoints[layerMode] ?? endpoints['rain']!;
    final url = await resolveUrl(path);

    final response = await http.get(Uri.parse(url));
    if (response.statusCode != 200) {
      throw Exception('Failed to load $layerMode metadata');
    }
    return ForecastMetadata.fromJson(jsonDecode(response.body));
  }

  static Future<TimeseriesResult> fetchTimeseries(String layerMode, String ens,
      double lat, double lon, int windHeight) async {
    String path = '';
    if (layerMode == 'temp') {
      path = '/api/timeseries/temp?lat=$lat&lon=$lon';
    } else if (layerMode == 'solar') {
      path = '/api/timeseries/solar?lat=$lat&lon=$lon';
    } else if (layerMode == 'wind') {
      path = '/api/timeseries/wind?lat=$lat&lon=$lon&height=$windHeight';
    } else {
      path = '/api/timeseries?ens=$ens&lat=$lat&lon=$lon';
    }

    final url = await resolveUrl(path);
    final response = await http.get(Uri.parse(url));
    if (response.statusCode != 200) {
      throw Exception('Failed to load timeseries');
    }
    return TimeseriesResult.fromJson(jsonDecode(response.body));
  }

  static Future<WindTimeseriesResult> fetchWindTimeseries(
      double lat, double lon, int windHeight) async {
    final path = '/api/timeseries/wind?lat=$lat&lon=$lon&height=$windHeight';
    final url = await resolveUrl(path);
    final response = await http.get(Uri.parse(url));
    if (response.statusCode != 200) {
      throw Exception('Failed to load wind timeseries');
    }
    return WindTimeseriesResult.fromJson(jsonDecode(response.body));
  }

  static Future<Uint8List> fetchImageBytes(String relativePath) async {
    final url = await resolveUrl(relativePath);
    final response = await http.get(Uri.parse(url));
    if (response.statusCode != 200) {
      throw Exception('Failed to download image from $relativePath');
    }
    return response.bodyBytes;
  }

  static Future<bool> testConnection(String url) async {
    String cleanUrl = url.trim();
    if (cleanUrl.endsWith('/')) {
      cleanUrl = cleanUrl.substring(0, cleanUrl.length - 1);
    }
    try {
      final response = await http
          .get(Uri.parse('$cleanUrl/api/metadata'))
          .timeout(const Duration(seconds: 4));
      return response.statusCode == 200;
    } catch (_) {
      return false;
    }
  }
}
