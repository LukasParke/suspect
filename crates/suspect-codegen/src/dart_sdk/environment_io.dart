import 'dart:io' show Platform;

/// Read only when a configured client takes its credential snapshot.
String? readVariable(String name) {
  try { return Platform.environment[name]; } on Object { return null; }
}
