// A selected operation set need not instantiate every serialization operand.
// ignore_for_file: unused_element, unused_element_parameter
enum _Location { path, query, querystring, header, cookie }
enum _Style { simple, label, matrix, form, spaceDelimited, pipeDelimited, deepObject, cookie }
enum _Encoding { component, reserved, none, form }
enum _Scalar { string, boolean, integer, number }
enum _Shape { scalar, array, object }
enum _MediaKind { json, text, bytes, form, multipart, sse, jsonl }

final class _Serialization {
  const _Serialization({this.style = _Style.simple, this.explode = false,
    this.encoding = _Encoding.component, this.shape = _Shape.scalar,
    this.scalar = _Scalar.string, this.properties = const {}, this.extra,
    this.anyExtra = false, this.content = false, this.jsonContent = false});
  final _Style style;
  final bool explode;
  final _Encoding encoding;
  final _Shape shape;
  final _Scalar scalar;
  final Map<String, _Scalar> properties;
  final _Scalar? extra;
  final bool anyExtra;
  final bool content;
  final bool jsonContent;
}

final class _Parameter {
  const _Parameter(this.name, this.location, this.required, this.serialization, {this.form});
  final String name;
  final _Location location;
  final bool required;
  final _Serialization serialization;
  final _FormSpec? form;
}

final class _Media {
  _Media(this.declared, this.kind, {this.maxBytes = 2147483647,
      this.scalar = _Scalar.string, this.maxItemBytes = 1048576})
      : type = _MediaType.parse(declared, ranges: true);
  final String declared;
  final _MediaKind kind;
  final int maxBytes;
  final _Scalar scalar;
  final int maxItemBytes;
  final _MediaType type;
}

final class _Response {
  const _Response(this.key, this.media);
  final String key;
  final List<_Media> media;
  int rank(int status) => key == '$status' ? 3 : key == '${status ~/ 100}XX' ? 2 : key == 'default' ? 1 : 0;
}

final class _WireOperation {
  const _WireOperation(this.source, this.method, this.path, this.servers,
      this.security, this.parameters, this.responses);
  final SchemaSource source;
  final String method;
  final String path;
  final List<ServerInfo> servers;
  final List<List<_Requirement>> security;
  final List<_Parameter> parameters;
  final List<_Response> responses;
  int response(int status) {
    var found = -1;
    var rank = 0;
    for (var i = 0; i < responses.length; i++) {
      final next = responses[i].rank(status);
      if (next > rank) { found = i; rank = next; }
    }
    return found;
  }
}

final class _MediaType {
  const _MediaType(this.type, this.subtype, this.parameters);
  final String type;
  final String subtype;
  final Map<String, String> parameters;
  int get specificity => (type == '*' ? 0 : subtype == '*' ? 1 : 2) * 10000 + parameters.length;
  bool matches(_MediaType actual) => (type == '*' || type == actual.type) &&
      (subtype == '*' || subtype == actual.subtype) && parameters.entries.every((entry) {
        final found = actual.parameters[entry.key];
        return found != null && (entry.key == 'charset' ? found.toLowerCase() == entry.value.toLowerCase() : found == entry.value);
      });
  factory _MediaType.parse(String text, {bool ranges = false}) {
    var at = 0;
    void space() { while (at < text.length && (text.codeUnitAt(at) == 32 || text.codeUnitAt(at) == 9)) { at++; } }
    String token() {
      final start = at;
      while (at < text.length && _tchar(text.codeUnitAt(at))) { at++; }
      return text.substring(start, at);
    }
    Never bad() => throw const ConfigurationException('invalid media type syntax');
    space(); final type = token().toLowerCase();
    if (type.isEmpty || at == text.length || text[at++] != '/') { bad(); }
    final subtype = token().toLowerCase();
    if (subtype.isEmpty || (!ranges && (type.contains('*') || subtype.contains('*'))) ||
        (type.contains('*') && type != '*') || (subtype.contains('*') && subtype != '*') ||
        (type == '*' && subtype != '*')) { bad(); }
    final parameters = <String, String>{};
    while (true) {
      space(); if (at == text.length) { break; }
      if (text[at++] != ';') { bad(); }
      space(); final key = token().toLowerCase();
      if (key.isEmpty || parameters.containsKey(key) || at == text.length || text[at++] != '=') { bad(); }
      String value;
      if (at < text.length && text[at] == '"') {
        at++; final out = StringBuffer(); var closed = false;
        while (at < text.length) {
          var c = text.codeUnitAt(at++);
          if (c == 34) { closed = true; break; }
          if (c == 92) { if (at == text.length) { bad(); } c = text.codeUnitAt(at++); }
          if ((c < 32 && c != 9) || c == 127 || c > 255) { bad(); }
          out.writeCharCode(c);
        }
        if (!closed) { bad(); } value = out.toString();
      } else { value = token(); if (value.isEmpty) { bad(); } }
      parameters[key] = value;
    }
    return _MediaType(type, subtype, Map.unmodifiable(parameters));
  }
}

int _selectMedia(List<_Media> choices, String contentType) {
  final actual = _MediaType.parse(contentType);
  var result = -1;
  var best = -1;
  for (var i = 0; i < choices.length; i++) {
    if (choices[i].type.matches(actual) && choices[i].type.specificity > best) {
      result = i; best = choices[i].type.specificity;
    }
  }
  if (result < 0) { throw const ConfigurationException('undeclared content type'); }
  final utf8Kind = switch(choices[result].kind) {
    _MediaKind.bytes || _MediaKind.json || _MediaKind.multipart => false,
    _MediaKind.text || _MediaKind.form || _MediaKind.sse || _MediaKind.jsonl => true,
  };
  if (utf8Kind && actual.parameters['charset'] != null &&
      actual.parameters['charset']!.toLowerCase() != 'utf-8') {
    throw const ConfigurationException('only UTF-8 is supported for structured content');
  }
  return result;
}

bool _tchar(int c) => c >= 65 && c <= 90 || c >= 97 && c <= 122 ||
    c >= 48 && c <= 57 || r"!#$%&'*+-.^_`|~".codeUnits.contains(c);
String _trimOws(String value) {
  var start = 0; var end = value.length;
  while (start < end && (value.codeUnitAt(start) == 32 || value.codeUnitAt(start) == 9)) { start++; }
  while (end > start && (value.codeUnitAt(end - 1) == 32 || value.codeUnitAt(end - 1) == 9)) { end--; }
  return value.substring(start, end);
}

int _scalarCompare(String a, String b) {
  final left = a.runes.iterator; final right = b.runes.iterator;
  while (true) {
    final l = left.moveNext(); final r = right.moveNext();
    if (!l || !r) { return l == r ? 0 : l ? 1 : -1; }
    if (left.current != right.current) { return left.current.compareTo(right.current); }
  }
}
String _scalarText(JsonValue value, [_Scalar? type]) {
  switch (value) {
    case JsonString(): if (type == null || type == _Scalar.string) { return value.value; }
    case JsonBoolean(): if (type == null || type == _Scalar.boolean) { return value.value ? 'true' : 'false'; }
    case JsonNumber(): if (type == null || type == _Scalar.number || type == _Scalar.integer && value.isInteger) { return value.token; }
    default: break;
  }
  throw const ConfigurationException('value has no declared scalar wire representation');
}

String _percent(String value, _Encoding mode, int ceiling) {
  if (value.length > ceiling || _unicodeLength(value) > ceiling) { throw const ResourceLimitException('encoded wire value'); }
  if (mode == _Encoding.none) { return value; }
  final bytes = utf8.encode(value); final out = StringBuffer(); var size = 0;
  for (var i = 0; i < bytes.length; i++) {
    final b = bytes[i];
    final ascii = b >= 65 && b <= 90 || b >= 97 && b <= 122 || b >= 48 && b <= 57;
    final unreserved = ascii || (mode == _Encoding.form ? '*-._' : '-._~').codeUnits.contains(b);
    if (mode == _Encoding.reserved && b == 37 && i + 2 < bytes.length && _hexDigit(bytes[i + 1]) && _hexDigit(bytes[i + 2])) {
      size += 3; if (size > ceiling) { throw const ResourceLimitException('encoded wire value'); }
      out.write(String.fromCharCodes(bytes.sublist(i, i + 3))); i += 2; continue;
    }
    final pass = unreserved || mode == _Encoding.reserved && r":/?#[]@!$&'()*+,;=".codeUnits.contains(b);
    size += pass || mode == _Encoding.form && b == 32 ? 1 : 3;
    if (size > ceiling) { throw const ResourceLimitException('encoded wire value'); }
    out.write(pass ? String.fromCharCode(b) : mode == _Encoding.form && b == 32 ? '+' : '%${b.toRadixString(16).padLeft(2,'0').toUpperCase()}');
  }
  return out.toString();
}
bool _hexDigit(int c) => c >= 48 && c <= 57 || c >= 65 && c <= 70 || c >= 97 && c <= 102;

final class _WireBuffer {
  _WireBuffer(this.limit);
  final int limit;
  final StringBuffer _out = StringBuffer();
  int length = 0;
  int get remaining => limit - length;
  void add(String value) {
    length += value.length;
    if (length > limit) { throw const ResourceLimitException('assembled wire value'); }
    _out.write(value);
  }
  void joined(Iterable<String> values, String delimiter) {
    var first = true;
    for (final value in values) { if (!first) { add(delimiter); } first = false; add(value); }
  }
  @override String toString() => _out.toString();
}

String _encodeWire(String value, _Serialization spec, _Location location, int ceiling) {
  if (spec.encoding == _Encoding.none) {
    if (value.codeUnits.any((c) => c < 32 && !(location == _Location.header && c == 9) || c == 127)) {
      throw const ConfigurationException('wire controls are not allowed');
    }
    if (location == _Location.cookie && value.codeUnits.any((c) => c > 126 || ' \t",;\\'.codeUnits.contains(c))) {
      throw const ConfigurationException('cookie value requires caller-defined escaping');
    }
  }
  if (!spec.content && (spec.style == _Style.spaceDelimited && value.contains(' ') ||
      spec.style == _Style.pipeDelimited && value.contains('|') || spec.style == _Style.deepObject && (value.contains('[') || value.contains(']')))) {
    throw const ConfigurationException('data contains an ambiguous style delimiter');
  }
  if (spec.encoding == _Encoding.reserved) {
    final hazards = switch (location) { _Location.path => '#[]/?', _Location.query || _Location.querystring => '#[]&=+', _Location.cookie => ';,', _ => '' };
    final separators = spec.shape == _Shape.scalar || spec.content ? '' : switch (spec.style) {
      _Style.simple || _Style.form || _Style.cookie => ',', _Style.label => '.,', _Style.matrix => ';,', _ => '' };
    if (value.codeUnits.any((c) => '$hazards$separators'.codeUnits.contains(c))) {
      throw const ConfigurationException('reserved expansion requires pre-escaped URI/data delimiters');
    }
  }
  return _percent(value, spec.encoding, ceiling);
}

String _serialize(_Parameter parameter, JsonValue value, int ceiling) {
  final s = parameter.serialization; final location = parameter.location;
  if (parameter.form != null) { return _formJson(parameter.form!, value, ceiling); }
  if (s.content) {
    final raw = s.jsonContent ? writeJson(value, limits: JsonLimits(maxBytes: ceiling, maxNumberBytes: _encodeLimits.maxNumberBytes)) : _scalarText(value);
    final encoded = _encodeWire(raw, s, location, ceiling);
    return location == _Location.query || location == _Location.cookie ?
        '${_percent(parameter.name,_Encoding.component,ceiling)}=$encoded' : encoded;
  }
  final name = _percent(parameter.name, location == _Location.header || s.style == _Style.cookie ? _Encoding.none : _Encoding.component, ceiling);
  final out = _WireBuffer(ceiling);
  String encode(String v) => _encodeWire(v, s, location, out.remaining);
  String? scalar;
  final items = <String>[];
  final properties = <(String,String)>[];
  switch (s.shape) {
    case _Shape.scalar: scalar = _scalarText(value, s.scalar);
    case _Shape.array:
      if (value is! JsonArray) { throw const ConfigurationException('expected a flat array parameter'); }
      if (value.values.isEmpty) { throw const ConfigurationException('empty composite has no value expansion'); }
      if (value.values.length > ceiling) { throw const ResourceLimitException('wire item visits'); }
      for (final item in value.values) { items.add(_scalarText(item,s.scalar)); }
    case _Shape.object:
      if (value is! JsonObject || value.values.isEmpty) { throw const ConfigurationException('expected a nonempty flat object parameter'); }
      if (value.values.length > ceiling) { throw const ResourceLimitException('wire member visits'); }
      final keys = value.values.keys.toList()..sort(_scalarCompare);
      for (final key in keys) {
        if (!s.properties.containsKey(key) && s.extra == null && !s.anyExtra) { throw const ConfigurationException('undeclared flat wire member'); }
        properties.add((key, _scalarText(value.values[key]!,s.properties[key] ?? s.extra)));
      }
  }
  void sequence(String delimiter, bool pairs) {
    if (scalar != null) { out.add(encode(scalar)); return; }
    if (s.shape == _Shape.array) { out.joined(items.map(encode),delimiter); return; }
    var first = true;
    for (final (key,value) in properties) {
      if (!first) { out.add(delimiter); } first = false;
      out.add(encode(key)); out.add(pairs ? '=' : delimiter); out.add(encode(value));
    }
  }
  switch (s.style) {
    case _Style.simple: sequence(',',s.explode);
    case _Style.label: out.add('.'); sequence(s.explode ? '.' : ',',s.explode);
    case _Style.matrix:
      if (scalar != null) { out.add(';$name'); if (scalar.isNotEmpty) { out.add('='); out.add(encode(scalar)); } }
      else if (!s.explode) { out.add(';$name='); sequence(',',false); }
      else if (s.shape == _Shape.array) {
        for (final item in items) { out.add(';$name'); if (item.isNotEmpty) { out.add('='); out.add(encode(item)); } }
      } else {
        for (final (key,value) in properties) { out.add(';'); out.add(encode(key)); if (value.isNotEmpty) { out.add('='); out.add(encode(value)); } }
      }
    case _Style.form:
    case _Style.cookie:
      final delimiter = s.style == _Style.cookie ? '; ' : '&';
      if (scalar != null || !s.explode) { out.add('$name='); sequence(',',false); }
      else if (s.shape == _Shape.array) {
        var first = true;
        for (final item in items) { if (!first) { out.add(delimiter); } first = false; out.add('$name='); out.add(encode(item)); }
      } else { sequence(delimiter,true); }
    case _Style.spaceDelimited: out.add('$name='); sequence('%20',false);
    case _Style.pipeDelimited: out.add('$name='); sequence('%7C',false);
    case _Style.deepObject:
      var first = true;
      for (final (key,value) in properties) {
        if (!first) { out.add('&'); } first = false;
        out.add('$name%5B'); out.add(encode(key)); out.add('%5D='); out.add(encode(value));
      }
  }
  return out.toString();
}

JsonValue _textJson(String value, _Scalar scalar) => switch (scalar) {
  _Scalar.string => JsonString(value),
  _Scalar.boolean => value == 'true' ? const JsonBoolean(true) : value == 'false' ? const JsonBoolean(false) : throw const JsonException('invalid textual boolean'),
  _Scalar.integer => JsonInteger.parse(value,maxBytes:_decodeLimits.maxNumberBytes),
  _Scalar.number => JsonNumber.parse(value,maxBytes:_decodeLimits.maxNumberBytes),
};

JsonValue? _headerJson(RawResponse response, _Parameter parameter) {
  final values = response.headers[parameter.name.toLowerCase()];
  if (values == null) {
    if (parameter.required) { throw InvalidResponseException(response); }
    return null;
  }
  final s = parameter.serialization;
  if (values.isEmpty || parameter.name.toLowerCase() == 'set-cookie' && values.length != 1 ||
      s.shape == _Shape.scalar && values.length != 1) { throw InvalidResponseException(response); }
  final text = values.join(',');
  try {
    if (s.content) { return s.jsonContent ? parseJson(text,limits:_decodeLimits) : _textJson(text,s.scalar); }
    if (s.shape == _Shape.scalar) { return _textJson(text,s.scalar); }
    final pieces = text.split(',').map(_trimOws).toList();
    if (s.shape == _Shape.array) { return JsonArray(pieces.map((v)=>_textJson(v,s.scalar))); }
    final fields = <String,JsonValue>{};
    for (var i = 0; i < pieces.length; i++) {
      String key; String value;
      if (s.explode) { final split = pieces[i].indexOf('='); if (split < 0) { throw const JsonException('invalid header object'); } key=pieces[i].substring(0,split); value=pieces[i].substring(split+1); }
      else { key=pieces[i]; if (++i==pieces.length) { throw const JsonException('invalid header object'); } value=pieces[i]; }
      if (fields.containsKey(key) || !s.properties.containsKey(key) && s.extra==null && !s.anyExtra) { throw const JsonException('invalid header member'); }
      fields[key]=_textJson(value,s.properties[key] ?? s.extra ?? _Scalar.string);
    }
    return JsonObject(fields);
  } on JsonException { throw InvalidResponseException(response); }
}
