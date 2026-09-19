import ai.openrouter.sdk.Client;
import ai.openrouter.sdk.SdkException;

/** Compiled support program for the explicitly configured OpenRouter env policy.
 * Native verification compiles this file; account requests are not run in tests.
 */
public final class GetCurrentKeyFromEnv {
    private GetCurrentKeyFromEnv() {}
    public static void main(String[] args) {
        try (var client = Client.fromEnv(); var response = client.getCurrentKey()) {
            System.out.println("GET /key HTTP " + response.status() + "; decoded=" + (response.data().data() != null));
        } catch (SdkException error) {
            System.err.println("getCurrentKey failed: " + error.kind() + " (HTTP " + error.status() + ")");
            System.exit(1);
        }
    }
}
