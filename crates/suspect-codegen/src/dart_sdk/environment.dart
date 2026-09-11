// Configured packages only. The omission value differs from every explicit input.
final class _OmittedCredentials extends Credentials {
  const _OmittedCredentials();
}
String? _snapshotEnvironment(String? Function(String) read,String name,int maximum) {
  try {
    final value=read(name);
    if(value==null||value.isEmpty||value.length>maximum||_unicodeLength(value)>maximum)return null;
    return value;
  } on Object { return null; }
}
String? _environmentCredential(String? value,{required bool bearer,required bool header}) {
  if(value==null)return null;
  if(header&&value.codeUnits.any((c)=>c<32&&c!=9||c==127||c>255))return null;
  if(bearer) {
    try { _bearerToken(value); } on ConfigurationException { return null; }
  }
  return value;
}
