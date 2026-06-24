import 'package:flutter_test/flutter_test.dart';
import 'package:nimbus/app_state.dart';

void main() {
  test('AppState initialization', () {
    final state = AppState();
    expect(state.currentLayerMode, 'rain');
  });
}
