package __PACKAGE__

import java.net.URI

/** Strict RFC 3986 resolution for physical document URLs. Encoded dots/slashes
 * and empty path segments stay encoded and significant. No logical ID is used.
 */
internal fun resolveDocumentServer(base: URI, reference: URI): URI {
    val scheme: String?
    val authority: String?
    val path: String
    val query: String?
    if (reference.scheme != null) {
        scheme = reference.scheme
        authority = reference.rawAuthority
        path = removeServerDots(reference.rawPath ?: "")
        query = reference.rawQuery
    } else {
        scheme = base.scheme
        if (reference.rawAuthority != null) {
            authority = reference.rawAuthority
            path = removeServerDots(reference.rawPath ?: "")
            query = reference.rawQuery
        } else {
            authority = base.rawAuthority
            val relative = reference.rawPath ?: ""
            if (relative.isEmpty()) {
                path = base.rawPath ?: ""
                query = reference.rawQuery ?: base.rawQuery
            } else {
                path = removeServerDots(if (relative.startsWith('/')) relative else {
                    val parent = base.rawPath ?: ""
                    (if (authority != null && parent.isEmpty()) "/" else parent.substring(0, parent.lastIndexOf('/') + 1)) + relative
                })
                query = reference.rawQuery
            }
        }
    }
    return URI(buildString {
        if (scheme != null) append(scheme).append(':')
        if (authority != null) append("//").append(authority)
        append(path)
        if (query != null) append('?').append(query)
        reference.rawFragment?.let { append('#').append(it) }
    })
}

private fun removeServerDots(path: String): String {
    var input = path
    val out = StringBuilder()
    fun parent() { out.setLength(out.lastIndexOf("/").coerceAtLeast(0)) }
    while (input.isNotEmpty()) {
        when {
            input.startsWith("../") -> input = input.substring(3)
            input.startsWith("./") -> input = input.substring(2)
            input.startsWith("/./") -> input = input.substring(2)
            input == "/." -> input = "/"
            input.startsWith("/../") -> { input = input.substring(3); parent() }
            input == "/.." -> { input = "/"; parent() }
            input == "." || input == ".." -> input = ""
            else -> {
                val end = input.indexOf('/', if (input.startsWith('/')) 1 else 0).let { if (it < 0) input.length else it }
                out.append(input.substring(0, end))
                input = input.substring(end)
            }
        }
    }
    return out.toString()
}
