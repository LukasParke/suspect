
// V3-only indexed metadata and scope. No URI resolution or acquisition occurs
// during evaluation; URI processing below is structural metadata admission.
internal data class ResourceCycle(val node: Int, val context: Int)

internal class IndexedResources(value: JsonObject, private val nodes: List<JsonObject>) {
    val records = value.array("resources").map { it as JsonObject }
    val scopes = value.array("nodeScopes").map { (it as JsonArray).values }
    init {
        require(scopes.size == nodes.size && records.size <= 4096)
        val sources = mutableSetOf<SourceLocation>()
        val aliases = mutableMapOf<String, Int>()
        for ((index, record) in records.withIndex()) {
            val source = record.source().also(::resourceLocation)
            require(sources.add(source)) { "duplicate resource source" }
            val kind = record.text("kind")
            require(kind in listOf("document", "openApiDocument", "schema"))
            val canonical = resourceUriKey(record.text("canonicalUri"))
            val base = record.text("baseUri")
            require(java.net.URI(base).rawFragment == null)
            val baseKey = resourceUriKey(base)
            require(resourceUriKey(record.text("canonicalUri").substringBefore('#')) == baseKey)
            require(base == baseKey && (kind == "openApiDocument" || canonical == baseKey))
            val declared = record.values.getValue("declarationSource")
            if (declared !== JsonNull) {
                val at = (declared as JsonObject).let { SourceLocation(it.text("document"), it.text("pointer")) }.also(::resourceLocation)
                require(kind != "document" && at == scopedChild(source, if (kind == "schema") "\$id" else "\$self"))
            } else require(source.pointer.isEmpty() && canonical == resourceUriKey(source.document))
            val names = record.array("aliases").map { (it as JsonString).value }
            require(names.toSet().size == names.size)
            val normalized = mutableSetOf<String>()
            for (name in names) {
                val key = resourceUriKey(name)
                require(aliases.put(key, index)?.let { it == index } ?: true) { "ambiguous resource alias" }
                normalized.add(key)
            }
            require(canonical in normalized && baseKey in normalized) { "missing resource aliases" }
        }
        val used = mutableSetOf<Int>()
        for ((index, scope) in scopes.withIndex()) {
            require(scope.size == 3)
            val resourceIndex = scopedIndex(scope[0])
            require(resourceIndex in records.indices)
            val record = records[resourceIndex]
            val boundary = record.source()
            val root = (scope[1] as JsonObject).let { SourceLocation(it.text("document"), it.text("pointer")) }.also(::resourceLocation)
            val source = nodes[index].source().also(::resourceLocation)
            require(resourceContains(boundary, root) && resourceContains(root, source))
            require(record.text("kind") != "schema" || boundary == root)
            val suffix = source.pointer.substring(boundary.pointer.length)
            val expected = record.text("baseUri") + if (suffix.isEmpty()) "" else "#" + resourceFragment(suffix)
            require((scope[2] as JsonString).value == expected) { "canonical node address mismatch" }
            used.add(resourceIndex)
        }
        require(used.size == records.size) { "unused resource record" }
        for ((index, record) in records.withIndex()) {
            val names = mutableSetOf<String>()
            for (raw in record.array("dynamicAnchors")) {
                val binding = (raw as JsonArray).values
                require(binding.size == 3)
                val name = (binding[0] as JsonString).value
                require(resourceAnchor(name) && names.add(name))
                val source = (binding[1] as JsonObject).let { SourceLocation(it.text("document"), it.text("pointer")) }.also(::resourceLocation)
                val target = scopedIndex(binding[2])
                require(target in nodes.indices && source == scopedChild(nodes[target].source(), "\$dynamicAnchor") && resource(target) == index)
            }
        }
    }
    fun resource(node: Int): Int = scopedIndex(scopes[node][0])
    fun checkReference(check: JsonObject) {
        val target = check.number("target")
        val initial = check.number("initialResource")
        require(target in nodes.indices && initial in records.indices && resource(target) == initial)
        val raw = check.values.getValue("anchor")
        if (raw !== JsonNull) {
            val name = (raw as JsonString).value
            require(resourceAnchor(name) && records[initial].array("dynamicAnchors").any {
                val binding = (it as JsonArray).values
                (binding[0] as JsonString).value == name && scopedIndex(binding[2]) == target
            })
        }
    }
}

internal class ResourceScope(private val program: IndexedResources) {
    private val stack = mutableListOf<Int>()
    private val entered = mutableSetOf<Int>()
    private val contexts = mutableMapOf<Pair<Int, Int>, Int>()
    var context: Int = 0
        private set
    fun enter(node: Int, spend: () -> Unit): Int? {
        val resource = program.resource(node)
        if (resource in entered) return null
        spend()
        val previous = context
        // Pair equality retains exact context identity even under hash collisions.
        context = contexts.getOrPut(previous to resource) { contexts.size + 1 }
        entered.add(resource)
        stack.add(resource)
        return previous
    }
    fun leave(previous: Int?) {
        if (previous != null) {
            entered.remove(stack.removeAt(stack.lastIndex))
            context = previous
        }
    }
    fun resolve(check: JsonObject, spend: () -> Unit): Int {
        val name = (check.values.getValue("anchor") as? JsonString)?.value ?: return check.number("target")
        for (resource in stack) {
            spend()
            for (raw in program.records[resource].array("dynamicAnchors")) {
                spend()
                val binding = (raw as JsonArray).values
                if ((binding[0] as JsonString).value == name) return scopedIndex(binding[2])
            }
        }
        return check.number("target")
    }
}

private fun resourceContains(parent: SourceLocation, child: SourceLocation): Boolean =
    parent.document == child.document && (parent.pointer == child.pointer || child.pointer.startsWith(parent.pointer + "/"))
private fun resourceLocation(source: SourceLocation) {
    require(java.net.URI(source.document).isAbsolute && java.net.URI(source.document).rawFragment == null)
    require(source.pointer.isEmpty() || source.pointer.startsWith('/'))
    var at = 0
    while (at < source.pointer.length) if (source.pointer[at++] == '~') require(at < source.pointer.length && source.pointer[at++] in "01")
}
private fun resourceAnchor(value: String): Boolean = value.isNotEmpty() && (value[0] in 'a'..'z' || value[0] in 'A'..'Z' || value[0] == '_') && value.all { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' || it in "_.-" }
private fun resourceFragment(value: String): String = buildString {
    Json.unicode(value)
    for (byte in value.toByteArray(Charsets.UTF_8)) {
        val n = byte.toInt() and 255
        val c = n.toChar()
        if (n in 48..57 || n in 65..90 || n in 97..122 || c in "-._~!$&'()*+,;=:@/?") append(c)
        else append('%').append("0123456789ABCDEF"[n ushr 4]).append("0123456789ABCDEF"[n and 15])
    }
}
private fun resourceDecode(value: String): String {
    val bytes = java.io.ByteArrayOutputStream()
    var at = 0
    while (at < value.length) {
        val c = value[at++]
        if (c == '%') {
            require(at + 1 < value.length)
            val hi = value[at++].digitToIntOrNull(16); val lo = value[at++].digitToIntOrNull(16)
            require(hi != null && lo != null); bytes.write(hi * 16 + lo)
        } else { require(c.code < 128); bytes.write(c.code) }
    }
    return Charsets.UTF_8.newDecoder().onMalformedInput(java.nio.charset.CodingErrorAction.REPORT).onUnmappableCharacter(java.nio.charset.CodingErrorAction.REPORT).decode(java.nio.ByteBuffer.wrap(bytes.toByteArray())).toString()
}
private fun resourceUriKey(value: String): String {
    require(value.all { it.code in 33..126 })
    val uri = java.net.URI(value)
    require(uri.isAbsolute)
    val fragment = resourceDecode(uri.rawFragment ?: "")
    val scheme = uri.scheme.lowercase()
    val document = if (uri.isOpaque) scheme + ":" + uri.rawSchemeSpecificPart else buildString {
        append(scheme).append(':')
        uri.rawAuthority?.let { authority ->
            append("//")
            val at = authority.lastIndexOf('@')
            if (at >= 0) append(authority.substring(0, at + 1))
            val host = authority.substring(at + 1)
            val end = if (host.startsWith('[')) host.indexOf(']') + 1 else host.lastIndexOf(':').let { if (it < 0) host.length else it }
            require(end > 0)
            append(host.substring(0, end).lowercase()).append(host.substring(end))
        }
        append(resourceRemoveDots(uri.rawPath ?: ""))
        uri.rawQuery?.let { append('?').append(it) }
    }
    return document + if (fragment.isEmpty()) "" else "#" + resourceFragment(fragment)
}
private fun resourceRemoveDots(path: String): String {
    var input = path
    val out = StringBuilder()
    while (input.isNotEmpty()) when {
        input.startsWith("../") -> input = input.substring(3)
        input.startsWith("./") -> input = input.substring(2)
        input.startsWith("/./") -> input = input.substring(2)
        input == "/." -> input = "/"
        input.startsWith("/../") || input == "/.." -> { input = if (input == "/..") "/" else input.substring(3); out.setLength(out.lastIndexOf("/").coerceAtLeast(0)) }
        input == "." || input == ".." -> input = ""
        else -> { val end = input.indexOf('/', if (input.startsWith('/')) 1 else 0).let { if (it < 0) input.length else it }; out.append(input.substring(0, end)); input = input.substring(end) }
    }
    return out.toString()
}
