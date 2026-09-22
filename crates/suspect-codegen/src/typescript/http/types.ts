import type { ModelCodec } from '../codecs.js';
import type { JsonNumber, JsonValue } from '../json.js';
import type { ValidationSource } from '../validation.js';

/** The Fetch-compatible transport used by generated operations. */
export type Fetch = (input: string | URL | Request, init?: RequestInit) => Promise<Response>;
/** A caller-supplied HTTP Basic credential. No charset negotiation is inferred. */
export interface BasicCredential { readonly username: string; readonly password: string; readonly encoding?: 'utf-8' | 'latin1' }
/** A complete caller-chosen Authorization field value for OAuth2/OIDC. */
export interface AuthorizationCredential { readonly authorization: string }
/** A source-bound credential callback. Returning a credential does not trigger acquisition or retry. */
export type CredentialProvider<T> = (context: CredentialContext) => T | Promise<T>;
/** A credential can be static or resolved explicitly for this one call. */
export type Credential<T> = T | CredentialProvider<T>;
/** Runtime credential vocabulary; generated Credentials types narrow each source scheme. */
export type CredentialValue = Credential<string | BasicCredential | AuthorizationCredential>;

/** Original source location including its byte span. */
export interface SourceLocation { readonly source: ValidationSource; readonly span: { readonly start: number; readonly end: number } }
/** Separate source declaration, terminal definition and reference hops. */
export interface Provenance {
    readonly use_site: SourceLocation; readonly terminal: SourceLocation; readonly references: readonly SourceLocation[];
    readonly use_site_resource?: ResourceContext | null; readonly terminal_resource?: ResourceContext | null;
    readonly reference_resources?: readonly (ResourceContext | null)[];
}
/** Logical addresses are metadata beside physical source ownership and spans. */
export interface ResourceContext {
    readonly source: SourceLocation; readonly resource: SourceLocation;
    readonly kind: 'document' | 'open-api-document' | 'schema';
    readonly canonical_uri: string; readonly base_uri: string; readonly scope_address: string;
    readonly base_source: SourceLocation | null; readonly schema_root: SourceLocation | null;
    readonly aliases: readonly string[];
}
/** Source-located metadata value. */
export interface Located<T> { readonly source: SourceLocation; readonly value: T }
/** Metadata passed to caller credential hooks, including scopes versus roles. */
export interface CredentialContext {
    readonly operationSource: ValidationSource;
    readonly operationId: string;
    readonly requirement: CredentialRequirement;
    readonly signal: AbortSignal;
    /** Selected API server base for relative OAuth/OIDC endpoint metadata. No acquisition is performed. */
    readonly effectiveServerURL: string;
}
/** An explicit server-array selection and literal variable overrides. */
export interface ServerChoice {
    readonly index?: number;
    readonly name?: string;
    readonly variables?: Readonly<Record<string, string>>;
    /** URL serving a local-file source document, required for its relative servers. */
    readonly documentURL?: string;
}
/** Reusable transport policy; generated ClientOptions supplies precise credential types. */
export interface ClientOptions<Auth extends object = Readonly<Record<string, CredentialValue>>> {
    readonly auth?: Auth;
    readonly serverURL?: string;
    readonly server?: ServerChoice;
    readonly fetch?: Fetch;
    readonly maxResponseBytes?: number;
    readonly maxRequestBytes?: number;
    readonly maxPartBytes?: number;
    readonly maxStreamItemBytes?: number;
    readonly maxStreamBufferBytes?: number;
    readonly maxStreamItems?: number;
    readonly maxErrorCaptureBytes?: number;
    /** Full User-Agent override. `null` suppresses the automatic attribution header entirely. */
    readonly userAgent?: string | null;
    /** Replaces the SDK identity token in the automatic attribution header: `<name>` or `<name>/<version>`. */
    readonly applicationId?: string;
}
/** Per-call choices and cancellation, never persisted on a reusable client. */
export interface CallOptions {
    readonly signal?: AbortSignal;
    readonly server?: ServerChoice;
    readonly securityAlternative?: number;
}
/** Explicit media choice for a multi-media or wildcard request body. */
export interface MediaBody<M extends string, T> { readonly contentType: M; readonly data: T }
/** A finite multipart value with caller-supplied part metadata. */
export interface Part<T, H extends object = Record<string, never>, M extends string = string> {
    readonly data: T;
    readonly headers?: H;
    readonly contentType?: M;
    readonly filename?: string;
}
/** Raw in-memory file data; filenames are metadata and never filesystem paths. */
export type BinaryPart = Uint8Array | Part<Uint8Array>;
/** Read-only view of error bytes. Each error-body access receives a detached snapshot. */
export type ReadonlyBytes = Readonly<Omit<Uint8Array, 'set' | 'fill' | 'copyWithin' | 'reverse' | 'sort'>>;
/** Recursively immutable declared error data; streaming values validate as they are consumed. */
export type ReadonlyResponseData<T> =
    T extends JsonNumber ? T :
    T extends Uint8Array ? ReadonlyBytes :
    T extends AsyncIterable<infer Item> ? AsyncIterable<ReadonlyResponseData<Item>> :
    T extends readonly (infer Item)[] ? readonly ReadonlyResponseData<Item>[] :
    T extends object ? { readonly [Key in keyof T]: ReadonlyResponseData<T[Key]> } : T;
/** One matched source response. Status is always the actual HTTP status. */
export interface ApiResponse<T, S extends number = number, M extends string | null = string, H extends object = Record<string, never>, L extends object = Record<string, never>, D extends string | null = M> {
    readonly status: S;
    /** Canonical declaration media for concrete declarations; concrete wire media for wildcards. */
    readonly contentType: M;
    /** Matched source declaration, including a literal wildcard range. Use this to narrow media unions. */
    readonly mediaType: D;
    /** Complete received Content-Type, including parameters, or null. */
    readonly rawContentType: string | null;
    readonly headers: Headers;
    readonly typedHeaders: H;
    readonly links: L;
    readonly data: T;
}
/** Stable non-API failure categories. */
export type SdkFailureKind = 'request-validation' | 'request-representation' | 'transport' | 'cancelled' | 'resource-limit' | 'unexpected-response' | 'response-decoding';
/** Source-linked failure created by this exact runtime instance. */
export interface SdkError extends Error {
    readonly kind: SdkFailureKind;
    readonly operationSource: ValidationSource;
    readonly source: ValidationSource | undefined;
}
/** Branded declared non-2xx response. Streaming bodies validate item-by-item. */
export interface DeclaredApiError<T = unknown, S extends number = number, M extends string | null = string, H extends object = Record<string, never>, L extends object = Record<string, never>, D extends string | null = M> extends Error {
    readonly kind: 'api-error';
    readonly operationSource: ValidationSource;
    readonly responseSource: ValidationSource;
    readonly response: ApiResponse<ReadonlyResponseData<T>, S, M, H, L, D>;
}
/** An undeclared status/media response with bounded diagnostic capture. */
export interface UnexpectedResponseError extends SdkError {
    readonly kind: 'unexpected-response'; readonly status: number; readonly contentType: string | null;
    readonly headers: Headers; readonly rawCapture: string; readonly truncated: boolean;
}
/** Declared content or a declared header that failed decoding. */
export interface ResponseDecodingError extends SdkError {
    readonly kind: 'response-decoding'; readonly status: number; readonly contentType: string | null;
    readonly headers: Headers; readonly rawCapture: string; readonly truncated: boolean;
}
/** A source-declared Link. Values and expressions are inert metadata. */
export interface LinkMetadata {
    readonly source: Provenance; readonly name: string;
    readonly target: { readonly kind: 'operation-id' | 'operation-ref'; readonly value: Located<string>; readonly operation: SourceLocation };
    readonly parameters: Readonly<Record<string, Located<JsonValue>>>;
    readonly request_body: Located<JsonValue> | null;
    readonly description: Located<string> | null;
    readonly server: ServerPlan | null;
}

// Version 1 read-only protocol descriptor vocabulary. These are typed projections
// of the shared Rust descriptors, never schemas reconstructed from generated code.
/** Source-defined HTTP parameter or credential attachment location. */
export type Location = 'path' | 'query' | 'querystring' | 'header' | 'cookie';
export type Scalar = 'string' | 'boolean' | 'integer' | 'number';
export type Style = 'simple' | 'label' | 'matrix' | 'form' | 'spaceDelimited' | 'pipeDelimited' | 'deepObject' | 'cookie';
export type PercentEncoding = 'uri-component' | 'reserved-expansion' | 'none' | 'form-url-encoded';
export type WireShape =
    | { readonly kind: 'scalar'; readonly scalar: Scalar }
    | { readonly kind: 'array'; readonly items: Scalar }
    | { readonly kind: 'flat-object'; readonly properties: Readonly<Record<string, Scalar>>; readonly additional: { readonly kind: 'forbidden' | 'any-scalar' } | { readonly kind: 'typed'; readonly scalar: Scalar } };
export interface SchemaUse { readonly id: ValidationSource; readonly source: Provenance }
export interface CodecRef { readonly schema: SchemaUse; readonly input: 'json' | 'text-scalar' }
export interface MediaType {
    readonly declared: string;
    readonly range: { readonly kind: 'any' } | { readonly kind: 'type'; readonly type_name: string } | { readonly kind: 'concrete'; readonly type_name: string; readonly subtype: string };
    readonly parameters: Readonly<Record<string, string>>;
}
export type Serialization =
    | { readonly kind: 'style'; readonly style: Style; readonly explode: boolean; readonly shape: WireShape; readonly percent_encoding: PercentEncoding }
    | { readonly kind: 'content'; readonly media_type: MediaType; readonly percent_encoding: PercentEncoding };
export interface ParameterPlan {
    readonly source: Provenance; readonly name: string; readonly location: Location;
    readonly required: boolean; readonly codec: CodecRef; readonly serialization: Serialization;
    readonly content_media: MediaPlan | null;
}
export interface HeaderPlan { readonly source: Provenance; readonly name: string; readonly required: boolean; readonly codec: CodecRef; readonly serialization: Serialization; readonly content_media: MediaPlan | null }
export interface ServerPlan {
    readonly source: Provenance | null; readonly default_from: SourceLocation | null;
    /** Physical retrieval document containing the server; logical $self/$id names do not relocate it. */
    readonly document_base: SourceLocation;
    readonly template: string; readonly name: Located<string> | null;
    readonly variables: readonly { readonly source: Provenance; readonly name: string; readonly default: Located<string>; readonly values: readonly Located<string>[] | null }[];
}
export interface OAuthFlow {
    readonly source: SourceLocation;
    readonly kind: 'implicit' | 'password' | 'clientCredentials' | 'authorizationCode' | 'deviceAuthorization';
    readonly authorization_url: Located<string> | null; readonly token_url: Located<string> | null;
    readonly refresh_url: Located<string> | null; readonly device_authorization_url: Located<string> | null;
    readonly scopes: Readonly<Record<string, Located<string>>>;
}
export interface CredentialRequirement {
    readonly source: SourceLocation; readonly name: string; readonly scheme: Provenance;
    readonly description: Located<string> | null;
    readonly permissions: { readonly kind: 'scopes' | 'roles'; readonly names: readonly Located<string>[] };
    readonly credential:
        | { readonly kind: 'bearer'; readonly bearer_format: Located<string> | null }
        | { readonly kind: 'basic' }
        | { readonly kind: 'api-key'; readonly location: Location; readonly name: Located<string> }
        | { readonly kind: 'o-auth2'; readonly flows: readonly OAuthFlow[]; readonly metadata_url: Located<string> | null }
        | { readonly kind: 'open-id-connect'; readonly discovery_url: Located<string> };
}
export type SecurityPlan =
    | { readonly kind: 'undeclared' | 'no-auth'; readonly source: SourceLocation }
    | { readonly kind: 'alternatives'; readonly source: SourceLocation; readonly alternatives: readonly { readonly source: SourceLocation; readonly requirements: readonly CredentialRequirement[] }[] };
export interface BytePolicy { readonly max_bytes: number; readonly declared_max_bytes: Located<number | JsonNumber> | null }
export interface ObjectRules { readonly schema: SchemaUse; readonly required: readonly Located<string>[]; readonly min_properties: Located<number | JsonNumber> | null; readonly max_properties: Located<number | JsonNumber> | null }
export interface PartPlan {
    readonly source: Provenance; readonly name: string | null; readonly schema: SchemaUse; readonly required: boolean;
    readonly multiplicity: 'one' | 'repeated-array-items'; readonly min_items: Located<number | JsonNumber> | null; readonly max_items: Located<number | JsonNumber> | null;
    readonly encoding_source: Provenance | null; readonly content_types: readonly MediaType[];
    readonly representation: PartRepresentation; readonly headers: readonly HeaderPlan[];
}
export type PartRepresentation =
    | { readonly kind: 'json'; readonly codec: CodecRef; readonly outer_encoding: PercentEncoding }
    | { readonly kind: 'text'; readonly codec: CodecRef; readonly scalar: Scalar; readonly outer_encoding: PercentEncoding }
    | { readonly kind: 'binary'; readonly bytes: BytePolicy }
    | { readonly kind: 'style'; readonly codec: CodecRef; readonly serialization: Serialization };
export type AdditionalParts = { readonly kind: 'forbidden' } | { readonly kind: 'allowed'; readonly part: PartPlan };
export interface FormPlan { readonly rules: ObjectRules; readonly fields: readonly PartPlan[]; readonly additional: AdditionalParts }
export type MultipartPlan =
    | { readonly kind: 'named'; readonly rules: ObjectRules; readonly parts: readonly PartPlan[]; readonly additional: AdditionalParts }
    | { readonly kind: 'positional'; readonly schema: SchemaUse; readonly prefix: readonly PartPlan[]; readonly items: AdditionalParts; readonly min_items: Located<number | JsonNumber> | null; readonly max_items: Located<number | JsonNumber> | null };
export interface StreamPlan { readonly source: SourceLocation; readonly framing: 'server-sent-events' | 'json-lines'; readonly item_codec: CodecRef; readonly max_item_bytes: number }
export type Representation =
    | { readonly kind: 'json'; readonly codec: CodecRef | null }
    | { readonly kind: 'text'; readonly codec: CodecRef | null; readonly scalar: Scalar; readonly encoding: 'utf8' }
    | { readonly kind: 'binary'; readonly schema: SchemaUse | null; readonly bytes: BytePolicy }
    | { readonly kind: 'form'; readonly form: FormPlan }
    | { readonly kind: 'multipart'; readonly multipart: MultipartPlan }
    | { readonly kind: 'stream'; readonly stream: StreamPlan };
export interface MediaPlan { readonly source: Provenance; readonly media_type: MediaType; readonly representation: Representation }
export interface BodyPlan { readonly source: Provenance; readonly required: boolean; readonly media: readonly MediaPlan[]; readonly limits: { readonly body: number; readonly part: number; readonly stream_item: number } }
export interface ResponsePlan {
    readonly source: Provenance; readonly status_key: string;
    readonly status: { readonly kind: 'exact' | 'range'; readonly value: number } | { readonly kind: 'default' };
    readonly media: readonly MediaPlan[]; readonly headers: readonly HeaderPlan[]; readonly links: readonly LinkMetadata[]; readonly max_body_bytes: number;
}
export interface ProtocolOperation {
    readonly source: Provenance; readonly method: string;
    readonly path: string; readonly servers: { readonly source: SourceLocation; readonly candidates: readonly ServerPlan[] };
    readonly security: SecurityPlan; readonly parameters: readonly ParameterPlan[]; readonly body: BodyPlan | null; readonly responses: readonly ResponsePlan[];
}
export interface RuntimeLimits {
    readonly response: number; readonly request: number; readonly part: number;
    readonly streamItem: number; readonly streamBuffer: number; readonly streamItems: number;
}
export interface OperationDescriptor<I> {
    readonly operationId: string; readonly source: ValidationSource; readonly wire: ProtocolOperation;
    readonly limits: RuntimeLimits; readonly inputMembers: readonly string[];
    readonly parameterMembers: readonly string[];
    readonly requestCodecs: Readonly<Record<string, ModelCodec<unknown>>>;
    readonly responseCodecs: Readonly<Record<string, ModelCodec<unknown>>>;
    readonly objectExtras: Readonly<Record<string, string>>;
    readonly taggedBody: boolean;
    readonly inputType?: I;
    /** ua/v1 attribution constants; `null` when generation supplied none. */
    readonly attribution: AttributionPlan | null;
}
/** Compiled ua/v1 attribution constants from generation time. */
export interface AttributionPlan {
    readonly template_version: string;
    readonly suspect_version: string;
    readonly sdk_name: string;
    readonly sdk_version: string;
    readonly spec_version: string;
    readonly language: string;
}
