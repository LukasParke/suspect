package __PACKAGE__

/** Runtime environment access used only by an explicitly configured client
 * creation/factory path. Implementations return null for unavailable variables.
 */
public fun interface CredentialEnvironment {
    /** Read one configured variable name. Credential values must not be logged. */
    public fun value(name: String): String?

    /** Default JVM environment access. Creating the reader does not read values. */
    public companion object {
        /** Reads the process environment when a client snapshots its credentials. */
        public val system: CredentialEnvironment get() = CredentialEnvironment { name -> java.lang.System.getenv(name) }
    }
}

internal class EnvironmentSnapshot(private val environment: CredentialEnvironment) {
    private val values = mutableMapOf<String, String?>()
    fun usable(name: String, validate: (String) -> Unit): String? {
        val value = if (values.containsKey(name)) values[name] else {
            val read = try { environment.value(name) }
            catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
            catch (_: Exception) { null }
            read?.takeIf { it.isNotEmpty() && it.length <= Json.MAX_BYTES }.also { values[name] = it }
        } ?: return null
        return try { validate(value); value }
        catch (cancelled: kotlinx.coroutines.CancellationException) { throw cancelled }
        catch (_: Exception) { null }
    }
}
