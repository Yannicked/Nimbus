import 'package:flutter_test/flutter_test.dart';
import 'package:nimbus/models.dart';

void main() {
  group('parseDouble', () {
    test('returns double when input is int', () {
      expect(parseDouble(1), 1.0);
    });

    test('returns double when input is double', () {
      expect(parseDouble(1.5), 1.5);
    });

    test('returns null when input is null', () {
      expect(parseDouble(null), null);
    });
  });

  group('ValueQueryResult', () {
    test('fromJson handles valid value', () {
      final json = {'value': 10.5, 'status': 'ok'};
      final result = ValueQueryResult.fromJson(json);
      expect(result.value, 10.5);
      expect(result.status, 'ok');
    });

    test('fromJson handles null value', () {
      final json = {'value': null, 'status': 'ok'};
      final result = ValueQueryResult.fromJson(json);
      expect(result.value, null);
      expect(result.status, 'ok');
    });
  });

  group('WindValueQueryResult', () {
    test('fromJson handles valid values', () {
      final json = {'speed': 5.0, 'direction': 180, 'status': 'ok'};
      final result = WindValueQueryResult.fromJson(json);
      expect(result.speed, 5.0);
      expect(result.direction, 180.0);
      expect(result.status, 'ok');
    });
  });

  group('TimeseriesResult', () {
    test('fromJson handles valid values', () {
      final json = {
        'times': [1, 2],
        'values': [10, 20.5],
        'status': 'ok'
      };
      final result = TimeseriesResult.fromJson(json);
      expect(result.times, [1, 2]);
      expect(result.values, [10.0, 20.5]);
      expect(result.status, 'ok');
    });
  });

  group('WindTimeseriesResult', () {
    test('fromJson handles valid values', () {
      final json = {
        'times': [1, 2],
        'speeds': [5, 6.5],
        'directions': [100, 200.5],
        'status': 'ok'
      };
      final result = WindTimeseriesResult.fromJson(json);
      expect(result.times, [1, 2]);
      expect(result.speeds, [5.0, 6.5]);
      expect(result.directions, [100.0, 200.5]);
      expect(result.status, 'ok');
    });
  });
}
