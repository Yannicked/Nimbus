class ForecastMetadata {
  final String referenceTimeStr;
  final List<int> times;
  final List<int> ensembles;
  final dynamic version;

  ForecastMetadata({
    required this.referenceTimeStr,
    required this.times,
    required this.ensembles,
    required this.version,
  });

  factory ForecastMetadata.fromJson(Map<String, dynamic> json) {
    return ForecastMetadata(
      referenceTimeStr: json['reference_time_str'] ?? '',
      times: List<int>.from(json['times'] ?? []),
      ensembles: List<int>.from(json['ensembles'] ?? []),
      version: json['version'] ?? 0,
    );
  }
}

class ValueQueryResult {
  final double? value;
  final String status;

  ValueQueryResult({this.value, required this.status});

  factory ValueQueryResult.fromJson(Map<String, dynamic> json) {
    return ValueQueryResult(
      value: json['value'] != null ? (json['value'] as num).toDouble() : null,
      status: json['status'] ?? '',
    );
  }
}

class WindValueQueryResult {
  final double? speed;
  final double? direction;
  final String status;

  WindValueQueryResult({this.speed, this.direction, required this.status});

  factory WindValueQueryResult.fromJson(Map<String, dynamic> json) {
    return WindValueQueryResult(
      speed: json['speed'] != null ? (json['speed'] as num).toDouble() : null,
      direction: json['direction'] != null
          ? (json['direction'] as num).toDouble()
          : null,
      status: json['status'] ?? '',
    );
  }
}

class TimeseriesResult {
  final List<int> times;
  final List<double> values;
  final String status;

  TimeseriesResult(
      {required this.times, required this.values, required this.status});

  factory TimeseriesResult.fromJson(Map<String, dynamic> json) {
    return TimeseriesResult(
      times: List<int>.from(json['times'] ?? []),
      values: List<double>.from(
          (json['values'] ?? []).map((v) => (v as num).toDouble())),
      status: json['status'] ?? '',
    );
  }
}

class WindTimeseriesResult {
  final List<int> times;
  final List<double> speeds;
  final List<double> directions;
  final String status;

  WindTimeseriesResult({
    required this.times,
    required this.speeds,
    required this.directions,
    required this.status,
  });

  factory WindTimeseriesResult.fromJson(Map<String, dynamic> json) {
    return WindTimeseriesResult(
      times: List<int>.from(json['times'] ?? []),
      speeds: List<double>.from(
          (json['speeds'] ?? []).map((v) => (v as num).toDouble())),
      directions: List<double>.from(
          (json['directions'] ?? []).map((v) => (v as num).toDouble())),
      status: json['status'] ?? '',
    );
  }
}
