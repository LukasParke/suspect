package {package};

import java.net.URI;
import java.time.Duration;
import java.util.*;

/** Immutable per-call choices. Every server/security index refers to source metadata. */
public final class RequestOptions {
    final URI server, documentUrl;
    final String serverName, accept;
    final Integer serverIndex, securityAlternative;
    final Map<String,String> variables;
    final Duration timeout;
    private RequestOptions(Builder b) {
        server=b.server;documentUrl=b.documentUrl;serverName=b.serverName;accept=b.accept;serverIndex=b.serverIndex;
        securityAlternative=b.securityAlternative;variables=Map.copyOf(b.variables);timeout=b.timeout;
        if(serverIndex!=null&&serverIndex<0||securityAlternative!=null&&securityAlternative<0||serverIndex!=null&&serverName!=null)throw new IllegalArgumentException("invalid source choice");
        if(timeout!=null&&(timeout.isZero()||timeout.isNegative()||timeout.compareTo(Duration.ofDays(1))>0))throw new IllegalArgumentException("invalid deadline");
        if(accept!=null)HttpWire.headerValue(accept);
    }
    /** Source defaults with no per-call override. @return defaults */
    public static RequestOptions defaults(){return builder().build();}
    /** Begin explicit overrides. @return builder */
    public static Builder builder(){return new Builder();}
    /** Mutable construction of an immutable request policy. */
    public static final class Builder {
        private URI server,documentUrl;private String serverName,accept;private Integer serverIndex,securityAlternative;private Duration timeout;
        private final Map<String,String> variables=new LinkedHashMap<>();
        private Builder(){}
        /** Explicit absolute server override. @param value URI @return builder */
        public Builder serverUrl(URI value){server=Objects.requireNonNull(value);return this;}
        /** HTTP retrieval URL for resolving a local specification's relative server. @param value URL @return builder */
        public Builder documentUrl(URI value){documentUrl=Objects.requireNonNull(value);return this;}
        /** Source server candidate by zero-based index. @param value index @return builder */
        public Builder serverIndex(int value){serverIndex=value;return this;}
        /** Source OAS 3.2 server name. @param value name @return builder */
        public Builder serverName(String value){serverName=Objects.requireNonNull(value);return this;}
        /** Literal source variable substitution. @param name declared variable @param value replacement @return builder */
        public Builder serverVariable(String name,String value){variables.put(Objects.requireNonNull(name),Objects.requireNonNull(value));return this;}
        /** Select an OR alternative explicitly, including an anonymous one. @param value index @return builder */
        public Builder securityAlternative(int value){securityAlternative=value;return this;}
        /** Whole-call/stream deadline override. @param value duration @return builder */
        public Builder timeout(Duration value){timeout=Objects.requireNonNull(value);return this;}
        /** Explicit Accept negotiation value. @param value header @return builder */
        public Builder accept(String value){accept=Objects.requireNonNull(value);return this;}
        /** Validate and snapshot. @return policy */
        public RequestOptions build(){return new RequestOptions(this);}
    }
}
