// Native TypeDoc conversion/rendering of actual declarations, followed by
// plan-derived symbol/source coverage and parsed-HTML text/link checks.
import assert from "node:assert/strict";
import { readFile, writeFile, readdir, mkdir, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";

const toolRoot = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(process.argv[2] ?? "");
const json = async (file) => JSON.parse(await readFile(file, "utf8"));
const own = await json(path.join(toolRoot, "package.json"));
assert.equal(process.versions.node, own.engines.node,
  `Native docs require Node ${own.engines.node}; select the pinned toolchain`);
const lockBytes = await readFile(path.join(toolRoot, "package-lock.json")).catch(() => {
  throw new Error("The documentation dependency lock is missing. Native docs remain blocked until pinned registry resolution and npm ci succeed.");
});
const lock = JSON.parse(lockBytes);
assert.equal(lock.lockfileVersion, 3, "Native docs require the reviewed npm v3 dependency lock");
assert.deepEqual(lock.packages[""].devDependencies, own.devDependencies, "The docs dependency lock is stale");
for (const [name, version] of Object.entries(own.devDependencies)) {
  let actual;
  try { actual = await json(path.join(toolRoot, "node_modules", name, "package.json")); }
  catch { throw new Error(`Missing pinned documentation tool ${name}@${version}. Install the docs toolchain before this native gate.`); }
  assert.equal(actual.version, version, `Unexpected ${name} version`);
}
assert.ok(process.argv[2], "usage: node build.mjs PATH_TO_GENERATED_TYPESCRIPT");
const { Application, ReflectionKind, Renderer } = await import("typedoc");
const { parse } = await import("parse5");
const manifest = await json(path.join(root, "docs-manifest.json"));
assert.equal(manifest.format, "suspect-typescript-docs-v1");
assert.equal(typeof manifest.codecsImplemented, "boolean", "The plan must explicitly declare codec support");
const codecsImplemented = manifest.codecsImplemented;
const httpImplemented = manifest.httpImplemented === true;
let httpManifest;
if (httpImplemented) {
  assert.equal(codecsImplemented, true, "HTTP documentation requires implemented model codecs");
  assert.equal(manifest.httpManifest, "http-manifest.json", "HTTP documentation requires its explicit manifest");
  httpManifest = await json(path.join(root, manifest.httpManifest));
  assert.equal(httpManifest.format, "suspect-typescript-http-v1");
  assert.equal(httpManifest.releaseReady, false);
  assert.ok(Array.isArray(httpManifest.operations) && httpManifest.operations.length > 0, "HTTP manifest requires selected operations");
  assert.equal(new Set(httpManifest.operations.map((operation) => operation.operationId)).size, httpManifest.operations.length, "Duplicate HTTP operation IDs");
  for (const operation of httpManifest.operations) {
    for (const field of ["export", "inputType", "successType", "errorType", "errorGuard"]) assert.equal(typeof operation[field], "string", `HTTP operation lacks ${field}`);
    assert.equal(typeof operation.hasSourceDescription, "boolean");
    assert.equal(typeof operation.descriptionText, "string");
    assert.ok(operation.source && typeof operation.source.document === "string" && typeof operation.source.pointer === "string", "HTTP operation lacks source identity");
    assert.ok(operation.security?.source && operation.security?.useSource && operation.security?.definitionSource, "HTTP operation lacks security provenance");
    assert.ok(Array.isArray(operation.responses) && operation.responses.length > 0, "HTTP operation lacks responses");
  }
} else {
  assert.ok(manifest.httpImplemented === undefined || manifest.httpImplemented === false, "Invalid HTTP support declaration");
  assert.ok(manifest.httpManifest === undefined, "HTTP manifest cannot be attached without implemented HTTP support");
}
assert.ok(Array.isArray(manifest.symbols), "Missing planned model symbols");
const plannedModels = new Map(manifest.symbols.map((symbol) => [symbol.name, symbol]));
assert.equal(plannedModels.size, manifest.symbols.length, "Duplicate planned model symbols");
if (manifest.symbols.length || codecsImplemented) assert.equal(manifest.releaseReady, false);
if (codecsImplemented) {
  assert.ok(Array.isArray(manifest.codecs), "Implemented codecs require a plan-derived codec manifest");
  assert.equal(manifest.codecs.length, manifest.symbols.length, "Every public model requires its planned codec");
  assert.equal(new Set(manifest.codecs.map((codec) => codec.name)).size, manifest.codecs.length, "Duplicate planned codec symbols");
  assert.equal(new Set(manifest.codecs.map((codec) => codec.model)).size, manifest.codecs.length, "A public model has duplicate codec entries");
  for (const codec of manifest.codecs) {
    const model = plannedModels.get(codec.model);
    assert.ok(model, `Codec ${codec.name} references an unplanned model`);
    assert.equal(codec.file, "model-codecs.ts", `Unexpected codec module for ${codec.name}`);
    assert.deepEqual(codec.source, model.source, `Codec/model source mismatch for ${codec.name}`);
  }
} else {
  assert.ok(manifest.codecs === undefined || Array.isArray(manifest.codecs) && manifest.codecs.length === 0,
    "Model-only documentation cannot claim public codecs");
}
const options = await json(path.join(root, "typedoc.json"));
if (codecsImplemented) {
  assert.deepEqual([...options.entryPoints].sort(), httpImplemented ? ["model-codecs.ts", "models.ts", "operations.ts"] : ["model-codecs.ts", "models.ts"],
    "Codec documentation must convert both actual generated entrypoints");
}
const output = path.join(root, "docs");
const htmlRoot = path.join(output, "html");
const app = await Application.bootstrapWithPlugins({
  ...options,
  entryPoints: options.entryPoints.map((file) => path.join(root, file)),
  tsconfig: path.join(root, options.tsconfig),
  readme: path.join(root, options.readme),
  out: htmlRoot,
  plugin: [],
});
const project = await app.convert();
assert.ok(project, "TypeDoc could not convert the generated declarations");
app.validate(project);
assert.ok(!app.logger.hasErrors() && !app.logger.hasWarnings(), "TypeDoc conversion/validation produced diagnostics");
await mkdir(output, { recursive: true });
// TypeDoc disposes its renderer router after the end event. Retain the native
// mapping built for this actual render instead of guessing filenames.
let nativeRouter;
app.renderer.on(Renderer.EVENT_END, () => { nativeRouter = app.renderer.router; });
await app.generateDocs(project, htmlRoot);
await app.generateJson(project, path.join(output, "reflections.json"));
assert.ok(!app.logger.hasErrors() && !app.logger.hasWarnings(), "TypeDoc rendering produced diagnostics");

const walk = function* (node) {
  yield node;
  for (const child of node.childNodes ?? []) yield* walk(child);
};
const text = (node) => [...walk(node)]
  .filter((child) => child.nodeName === "#text" && child.parentNode?.tagName !== "script" && child.parentNode?.tagName !== "style")
  .map((child) => child.value).join("");
const normalized = (value) => value.replace(/\s+/gu, " ").trim();
const files = async function* (directory) {
  for (const entry of (await readdir(directory, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) yield* files(file);
    else yield file;
  }
};
const documents = new Map();
for await (const file of files(htmlRoot)) {
  if (!file.endsWith(".html")) continue;
  const tree = parse(await readFile(file, "utf8"));
  const ids = new Set();
  for (const node of walk(tree)) {
    const attrs = Object.fromEntries((node.attrs ?? []).map(({ name, value }) => [name, value]));
    if (attrs.id) ids.add(attrs.id);
    assert.ok(!["iframe", "object", "embed", "img"].includes(node.tagName), `Unexpected active content in ${file}`);
    for (const name of Object.keys(attrs)) assert.ok(!/^on/iu.test(name), `Inline event handler in ${file}`);
    for (const name of ["href", "src", "action"]) {
      if (!attrs[name]) continue;
      assert.ok(!/^(?:javascript|data|vbscript):/iu.test(attrs[name].trim()), `Unsafe ${name} in ${file}`);
    }
  }
  documents.set(file, { tree, ids, text: normalized(text(tree)) });
}
assert.ok(documents.size > 0, "TypeDoc emitted no HTML documents");
if (manifest.validationVersion === "suspect.validation.experimental.v2") {
  assert.equal(manifest.validationProfile, "oas31-jsonschema202012-static-applicators");
  assert.ok([...documents.values()].some((document) => document.text.includes("Scoped v2 validation") &&
    document.text.includes("encode validates the current wire value again after mutation")), "Missing rendered scoped-validator and mutability contract");
}
if (manifest.validationVersion === "suspect.validation.experimental.v3") {
  assert.equal(manifest.validationProfile, "oas31-jsonschema202012-resources-dynamic");
  assert.ok([...documents.values()].some((document) => document.text.includes("Resources and dynamic references") &&
    document.text.includes("The runtime performs no source loading")), "Missing rendered resource/dynamic native contract");
}
let localLinks = 0;
for (const [file, document] of documents) {
  for (const node of walk(document.tree)) {
    const href = node.attrs?.find((attr) => attr.name === "href")?.value;
    if (!href || /^(?:[a-z][a-z0-9+.-]*:|\/\/)/iu.test(href)) continue;
    const url = new URL(href, `https://docs.invalid/${path.relative(htmlRoot, file).split(path.sep).join("/")}`);
    const target = path.resolve(htmlRoot, `.${decodeURIComponent(url.pathname)}`);
    assert.ok(target.startsWith(`${htmlRoot}${path.sep}`), `Link escaped documentation root: ${href}`);
    const info = await stat(target).catch(() => null);
    assert.ok(info?.isFile(), `Broken local link ${href} in ${file}`);
    if (url.hash && documents.has(target)) {
      assert.ok(documents.get(target).ids.has(decodeURIComponent(url.hash.slice(1))), `Broken anchor ${href} in ${file}`);
    }
    localLinks++;
  }
}
const aliases = project.getReflectionsByKind(ReflectionKind.TypeAlias);
const router = nativeRouter;
assert.ok(router, "TypeDoc did not retain its native output router");
const coverage = [];
const modelReflections = new Map();
for (const symbol of manifest.symbols) {
  const matches = aliases.filter((alias) => alias.name === symbol.name
    && (!codecsImplemented || alias.parent?.kindOf(ReflectionKind.Module) && alias.parent.name === "models"));
  assert.equal(matches.length, 1, `Missing/ambiguous TypeDoc model ${symbol.name}`);
  const reflection = matches[0];
  modelReflections.set(symbol.name, reflection);
  assert.ok(router.hasUrl(reflection), `No native documentation URL for ${symbol.name}`);
  const url = router.getFullUrl(reflection);
  const relative = url.split("#")[0];
  const page = documents.get(path.join(htmlRoot, decodeURIComponent(relative)));
  assert.ok(page, `Missing native HTML page for ${symbol.name}`);
  const source = `${symbol.source.document}#${symbol.source.pointer}`;
  assert.ok(page.text.includes(normalized(source)), `Missing or changed schema origin for ${symbol.name}`);
  const codecNotice = codecsImplemented
    ? "A validated model codec is implemented. HTTP transport is outside this model module."
    : "No codec or HTTP client is implemented";
  assert.ok(page.text.includes(codecNotice), `Missing or incorrect codec status for ${symbol.name}`);
  if (symbol.hasSourceDescription) {
    assert.ok(page.text.includes(normalized(symbol.descriptionText)), `Source prose was reinterpreted for ${symbol.name}`);
  }
  const remarks = reflection.comment?.blockTags?.filter((tag) => tag.tag === "@remarks") ?? [];
  assert.ok(remarks.length > 0, `Missing declaration remarks for ${symbol.name}`);
  coverage.push({ name: symbol.name, source: symbol.source, view: symbol.view, url,
    hasSourceDescription: symbol.hasSourceDescription, documented: true });
}
const codecCoverage = [];
if (codecsImplemented) {
  const resolveReflection = (reflection) => reflection?.kindOf(ReflectionKind.Reference)
    ? reflection.tryGetTargetReflectionDeep() : reflection;
  const location = (reflection, name) => {
    assert.ok(reflection && router.hasUrl(reflection), `Missing native documentation URL for ${name}`);
    const url = router.getFullUrl(reflection);
    const [relative, anchor] = url.split("#");
    const file = path.join(htmlRoot, decodeURIComponent(relative));
    const page = documents.get(file);
    assert.ok(page, `Missing native HTML page for ${name}`);
    if (anchor) assert.ok(page.ids.has(decodeURIComponent(anchor)), `Missing native HTML anchor for ${name}`);
    return { url, file, page };
  };
  const linked = (from, target) => {
    const expected = new URL(target, "https://docs.invalid/").href;
    return [...walk(from.page.tree)].some((node) => {
      const href = node.attrs?.find((attr) => attr.name === "href")?.value;
      return href && new URL(href, new URL(from.url, "https://docs.invalid/")).href === expected;
    });
  };
  const commentText = (parts) => normalized((parts ?? []).map((part) => part.text).join(""));
  // Follow actual TypeDoc aliases, preserving each generic type argument. A
  // missing target is a failure, never permission to infer methods from text.
  const codecType = (type) => {
    const seen = new Set();
    let current = type;
    while (current?.type === "reference") {
      const declaration = resolveReflection(current.reflection);
      assert.ok(declaration, "A codec API type was omitted from native documentation");
      assert.ok(!seen.has(declaration.id), "Cyclic codec API alias in native documentation");
      seen.add(declaration.id);
      if (declaration.kindOf(ReflectionKind.Interface)) return { declaration, arguments: current.typeArguments ?? [] };
      assert.ok(declaration.kindOf(ReflectionKind.TypeAlias), "A codec must reference its documented interface");
      const parameters = declaration.typeParameters ?? [];
      const argumentsById = new Map(parameters.map((parameter, index) => [parameter.id, current.typeArguments?.[index]]));
      const target = declaration.type;
      assert.equal(target?.type, "reference", "A codec alias must retain its documented API target");
      current = {
        type: "reference", reflection: target.reflection,
        typeArguments: (target.typeArguments ?? []).map((argument) =>
          argument.type === "reference" && argumentsById.has(argument.reflection?.id)
            ? argumentsById.get(argument.reflection.id) : argument),
      };
    }
    assert.fail("A codec has no documented generic interface");
  };
  const variables = project.getReflectionsByKind(ReflectionKind.Variable);
  const documentedCodecIds = new Set();
  const apiIds = new Set();
  for (const codec of manifest.codecs) {
    const matches = variables.filter((variable) => variable.name === codec.name
      && variable.parent?.kindOf(ReflectionKind.Module) && variable.parent.name === "model-codecs");
    assert.equal(matches.length, 1, `Missing/ambiguous TypeDoc codec ${codec.name}`);
    const reflection = matches[0];
    documentedCodecIds.add(reflection.id);
    const native = location(reflection, codec.name);
    const source = `${codec.source.document}#${codec.source.pointer}`;
    assert.ok(native.page.text.includes(normalized(source)), `Missing or changed schema origin for ${codec.name}`);
    assert.ok(native.page.text.includes("HTTP transport is outside this codec module"), `Missing HTTP support boundary for ${codec.name}`);
    assert.ok((reflection.comment?.blockTags ?? []).some((tag) => tag.tag === "@remarks" && commentText(tag.content)),
      `Missing declaration remarks for ${codec.name}`);
    const model = plannedModels.get(codec.model);
    if (model.hasSourceDescription) {
      assert.ok(native.page.text.includes(normalized(model.descriptionText)), `Source prose was reinterpreted for ${codec.name}`);
    }
    const api = codecType(reflection.type);
    apiIds.add(api.declaration.id);
    assert.equal(api.arguments.length, 1, `Codec ${codec.name} lost its model type argument`);
    assert.equal(resolveReflection(api.arguments[0]?.reflection)?.id, modelReflections.get(codec.model).id,
      `Codec ${codec.name} is not bound to its planned model type`);
    const parameters = api.declaration.typeParameters ?? [];
    assert.equal(parameters.length, 1, `Codec ${codec.name} must retain the model type parameter`);
    const apiLocation = location(api.declaration, `${codec.name} API`);
    const modelLocation = location(modelReflections.get(codec.model), codec.model);
    assert.ok(linked(native, modelLocation.url), `Codec ${codec.name} does not link its model documentation`);
    // The variable may link a public alias first; that alias must itself link
    // the resolved interface instead of hiding the callable API.
    const publicType = location(resolveReflection(reflection.type.reflection), `${codec.name} public API`);
    assert.ok(linked(native, publicType.url), `Codec ${codec.name} does not link its API documentation`);
    if (publicType.url !== apiLocation.url) {
      assert.ok(linked(publicType, apiLocation.url), `Codec ${codec.name} API alias does not link its interface`);
    }
    const methods = {};
    for (const name of ["decode", "encode"]) {
      const method = api.declaration.getChildByName(name);
      assert.ok(method?.kindOf(ReflectionKind.Method), `Missing documented ${codec.name}.${name} method`);
      assert.equal(method.signatures?.length, 1, `Missing/ambiguous ${codec.name}.${name} signature`);
      const signature = method.signatures[0];
      assert.equal(signature.typeParameters?.length ?? 0, 0, `${codec.name}.${name} shadows the model type`);
      const args = signature.parameters ?? [];
      assert.equal(args.length, 2, `${codec.name}.${name} lost its input or limits parameter`);
      assert.equal(args[0].name, name === "decode" ? "text" : "value");
      assert.equal(args[0].flags.isOptional, false);
      assert.equal(args[1].name, "options");
      assert.equal(args[1].flags.isOptional, true);
      const modelType = name === "decode" ? signature.type : args[0].type;
      const stringType = name === "decode" ? args[0].type : signature.type;
      assert.equal(resolveReflection(modelType?.reflection)?.id, parameters[0].id,
        `${codec.name}.${name} does not preserve its model type`);
      assert.ok(stringType?.type === "intrinsic" && stringType.name === "string",
        `${codec.name}.${name} lost its JSON text type`);
      const limits = resolveReflection(args[1].type?.reflection);
      assert.ok(limits?.name === "JsonLimits", `${codec.name}.${name} has no documented JsonLimits`);
      location(limits, `${codec.name}.${name} limits`);
      assert.ok(commentText(signature.comment?.summary), `Missing method prose for ${codec.name}.${name}`);
      for (const arg of args) {
        assert.ok(commentText(arg.comment?.summary), `Missing ${codec.name}.${name} parameter documentation for ${arg.name}`);
      }
      for (const tag of ["@returns", "@throws"]) {
        assert.ok((signature.comment?.blockTags ?? []).some((entry) => entry.tag === tag && commentText(entry.content)),
          `Missing ${tag} documentation for ${codec.name}.${name}`);
      }
      const methodLocation = location(method, `${codec.name}.${name}`);
      assert.ok(methodLocation.page.text.includes(commentText(signature.comment.summary)),
        `Native HTML omitted ${codec.name}.${name} documentation`);
      methods[name] = { url: methodLocation.url, signature: signature.toString(), documented: true };
    }
    codecCoverage.push({ name: codec.name, model: codec.model, source: codec.source, file: codec.file,
      url: native.url, modelUrl: modelLocation.url, apiUrl: apiLocation.url, methods, documented: true });
  }
  // An exported ModelCodec-valued variable omitted from the manifest cannot
  // disappear from coverage merely because the manifest forgot to list it.
  for (const variable of variables) {
    if (variable.type?.type !== "reference") continue;
    const target = resolveReflection(variable.type.reflection);
    const relevant = target && (apiIds.has(target.id) || target.name === "ModelCodec");
    if (relevant) assert.ok(documentedCodecIds.has(variable.id), `Unmanifested public codec ${variable.name}`);
  }
}
const operationCoverage = [];
if (httpImplemented) {
  // TypeDoc erases intrinsic aliases on interface members. Resolve the written
  // type annotation with TypeScript's checker so equal primitive shapes cannot
  // conceal a binding to a different source model.
  const { default: ts } = await import("typescript");
  const configFile = ts.readConfigFile(path.join(root, options.tsconfig), ts.sys.readFile);
  assert.equal(configFile.error, undefined, "HTTP compiler configuration could not be read");
  const compilerConfig = ts.parseJsonConfigFileContent(configFile.config, ts.sys, root);
  assert.equal(compilerConfig.errors.length, 0, "HTTP compiler configuration is invalid");
  const compilerProgram = ts.createProgram(compilerConfig.fileNames, compilerConfig.options);
  const checker = compilerProgram.getTypeChecker();
  const operationSource = compilerProgram.getSourceFile(path.join(root, "operations.ts"));
  const modelSource = compilerProgram.getSourceFile(path.join(root, "models.ts"));
  assert.ok(operationSource && modelSource, "HTTP model or operation compiler source is missing");
  const inputDeclarations = new Map(operationSource.statements.filter(ts.isInterfaceDeclaration).map((node) => [node.name.text, node]));
  const resolve = (reflection) => reflection?.kindOf(ReflectionKind.Reference) ? reflection.tryGetTargetReflectionDeep() : reflection;
  const commentText = (parts) => normalized((parts ?? []).map((part) => part.text).join(""));
  const moduleNamed = (reflection, name) => reflection.parent?.kindOf(ReflectionKind.Module) && reflection.parent.name === name;
  const one = (kind, name) => {
    const matches = project.getReflectionsByKind(kind).filter((reflection) => reflection.name === name && moduleNamed(reflection, "operations"));
    assert.equal(matches.length, 1, `Missing/ambiguous TypeDoc HTTP declaration ${name}`);
    return matches[0];
  };
  const native = (reflection, name) => {
    assert.ok(router.hasUrl(reflection), `No native documentation URL for ${name}`);
    const url = router.getFullUrl(reflection); const [relative, anchor] = url.split("#");
    const file = path.join(htmlRoot, decodeURIComponent(relative)); const page = documents.get(file);
    assert.ok(page, `Missing native HTML page for ${name}`);
    if (anchor) assert.ok(page.ids.has(decodeURIComponent(anchor)), `Missing native HTML anchor for ${name}`);
    return { url, page };
  };
  const target = (type) => resolve(type?.reflection);
  const nativeShape = (type, seen = new Set()) => {
    assert.ok(type, "Missing native input type");
    if (type.type === "reference") {
      const declaration = target(type);
      assert.ok(declaration, "Unresolved native input reference");
      // Object aliases are reflected through children with no .type. Preserve
      // their declaration identity rather than descending into an absent type.
      if (declaration.kindOf(ReflectionKind.TypeAlias) && declaration.type && !(type.typeArguments?.length) && !seen.has(declaration.id)) {
        return nativeShape(declaration.type, new Set([...seen, declaration.id]));
      }
      return ["reference", declaration.id, (type.typeArguments ?? []).map((argument) => nativeShape(argument, seen))];
    }
    if (type.type === "intrinsic") return ["intrinsic", type.name];
    if (type.type === "literal") return ["literal", type.value];
    if (type.type === "array") return ["array", nativeShape(type.elementType, seen)];
    if (type.type === "union" || type.type === "intersection") return [type.type, type.types.map((member) => nativeShape(member, seen))];
    if (type.type === "reflection") {
      const declaration = type.declaration;
      assert.ok(declaration && !(declaration.signatures?.length), "Unexpected callable native value carrier");
      return ["object",
        (declaration.children ?? []).map((member) => [member.name, !!member.flags.isOptional, !!member.flags.isReadonly, nativeShape(member.type, seen)]).sort((a, b) => a[0].localeCompare(b[0])),
        (declaration.indexSignatures ?? []).map((signature) => [!!signature.flags.isReadonly, (signature.parameters ?? []).map((parameter) => nativeShape(parameter.type, seen)), nativeShape(signature.type, seen)]),
      ];
    }
    assert.fail(`Unsupported erased native input type ${type.type}`);
  };
  const typeWalk = function* (type) {
    if (!type || typeof type !== "object") return;
    yield type;
    for (const key of ["types", "typeArguments", "elements"]) for (const child of type[key] ?? []) yield* typeWalk(child);
    for (const key of ["elementType", "targetType", "checkType", "extendsType", "trueType", "falseType", "objectType", "indexType"]) if (type[key]) yield* typeWalk(type[key]);
    for (const declaration of type.declaration?.children ?? []) yield* typeWalk(declaration.type);
  };
  const references = (type) => new Set([...typeWalk(type)].filter((part) => part.type === "reference").map((part) => resolve(part.reflection)?.id).filter(Boolean));
  const literals = (type) => [...typeWalk(type)].filter((part) => part.type === "literal").map((part) => part.value);
  const functions = project.getReflectionsByKind(ReflectionKind.Function).filter((reflection) => moduleNamed(reflection, "operations"));
  const plannedCodecs = new Map(manifest.codecs.map((codec) => [codec.model, codec.name]));
  const clientOptions = one(ReflectionKind.Interface, "ClientOptions");
  const credentialLiterals = (clientOptions.extendedTypes ?? []).flatMap((type) => literals(type));
  const permittedFunctions = new Set(["createClient", "isSdkError", ...httpManifest.operations.flatMap((operation) => [operation.export, operation.errorGuard])]);
  for (const fn of functions) assert.ok(permittedFunctions.has(fn.name), `Unmanifested public operation function ${fn.name}`);
  for (const name of permittedFunctions) assert.equal(functions.filter((fn) => fn.name === name).length, 1, `Missing/ambiguous public operation function ${name}`);
  for (const operation of httpManifest.operations) {
    if (operation.security.schemeName) {
      assert.ok(credentialLiterals.includes(operation.security.schemeName), `ClientOptions lost credential key ${operation.security.schemeName}`);
    } else {
      const security = operation.security.plan;
      assert.ok(security && (security.kind === "undeclared" || security.kind === "no-auth" ||
        security.kind === "alternatives" && security.alternatives.some((alternative) => alternative.requirements.length === 0)),
        "Missing credential binding without a source-declared anonymous policy");
    }
    for (const response of operation.responses) assert.equal(response.codec, plannedCodecs.get(response.model), `${operation.operationId} response codec/model binding changed`);
    const input = one(ReflectionKind.Interface, operation.inputType);
    const inputBindings = [...operation.parameters, ...(operation.body ? [operation.body] : [])];
    assert.deepEqual((input.children ?? []).map((member) => member.name).sort(), inputBindings.map((binding) => binding.member).sort(), `${operation.inputType} input members drifted`);
    for (const binding of inputBindings) {
      const member = input.children.find((child) => child.name === binding.member);
      assert.equal(member.flags.isOptional, !binding.required, `${operation.inputType}.${binding.member} requiredness drifted`);
      const model = modelReflections.get(binding.model);
      assert.ok(model, `${operation.inputType}.${binding.member} has no planned model`);
      const declaration = inputDeclarations.get(operation.inputType)?.members.find((node) => ts.isPropertySignature(node) && node.name.text === binding.member);
      assert.ok(declaration?.type && ts.isTypeReferenceNode(declaration.type), `${operation.inputType}.${binding.member} lost its named model annotation`);
      let symbol = checker.getSymbolAtLocation(declaration.type.typeName);
      if (symbol && (symbol.flags & ts.SymbolFlags.Alias)) symbol = checker.getAliasedSymbol(symbol);
      assert.ok(symbol?.declarations?.some((node) => ts.isTypeAliasDeclaration(node) && node.name.text === binding.model && node.getSourceFile() === modelSource), `${operation.inputType}.${binding.member} compiler model binding drifted`);
      if (target(member.type)?.id !== model.id) {
        assert.deepEqual(nativeShape(member.type), nativeShape(model.type), `${operation.inputType}.${binding.member} native model shape drifted`);
      }
      assert.equal(binding.codec, plannedCodecs.get(binding.model), `${operation.inputType}.${binding.member} codec binding drifted`);
      if (binding.location) {
        const page = native(member, `${operation.inputType}.${binding.member}`);
        assert.ok(page.page.text.includes(normalized(`${binding.source.document}#${binding.source.pointer}`)), `Missing parameter origin for ${binding.member}`);
        assert.ok(commentText(member.comment?.summary).includes(`${binding.location} parameter`), `Missing parameter location for ${binding.member}`);
        assert.ok(commentText(member.comment?.summary).includes(`style=${binding.style}, explode=${binding.explode}`), `Parameter serialization drifted for ${binding.member}`);
      }
    }
    const success = one(ReflectionKind.TypeAlias, operation.successType);
    const error = one(ReflectionKind.TypeAlias, operation.errorType);
    const successType = success.type ?? { type: "reflection", declaration: success };
    const errorType = error.type ?? { type: "reflection", declaration: error };
    const fn = one(ReflectionKind.Function, operation.export);
    const guard = one(ReflectionKind.Function, operation.errorGuard);
    const signature = fn.signatures?.[0];
    assert.ok(signature && fn.signatures.length === 1, `Missing/ambiguous signature for ${operation.export}`);
    const args = signature.parameters ?? [];
    assert.deepEqual(args.map((arg) => arg.name), ["client", "input", "call"], `${operation.export} parameter names changed`);
    assert.equal(args[0].flags.isOptional, false); assert.equal(args[1].flags.isOptional, false); assert.equal(args[2].flags.isOptional, true);
    assert.equal(target(args[1].type)?.id, input.id, `${operation.export} is not bound to ${operation.inputType}`);
    assert.equal(target(signature.type)?.name ?? signature.type?.name, "Promise", `${operation.export} must return Promise`);
    assert.equal(resolve(signature.type?.typeArguments?.[0]?.reflection)?.id, success.id, `${operation.export} Promise lost ${operation.successType}`);
    for (const arg of args) assert.ok(commentText(arg.comment?.summary), `Missing ${operation.export} @param ${arg.name}`);
    for (const tag of ["@returns", "@throws", "@remarks"]) assert.ok((signature.comment?.blockTags ?? []).some((entry) => entry.tag === tag && commentText(entry.content)), `Missing ${tag} for ${operation.export}`);
    const guardSignature = guard.signatures?.[0]; const guardArgs = guardSignature?.parameters ?? [];
    assert.deepEqual(guardArgs.map((arg) => arg.name), ["error"], `${operation.errorGuard} parameter changed`);
    assert.equal(guardSignature?.type?.type, "predicate", `${operation.errorGuard} must return a type predicate`);
    const guardDeclaration = operationSource.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === operation.errorGuard);
    const predicate = guardDeclaration?.type;
    assert.ok(predicate && ts.isTypePredicateNode(predicate) && predicate.type && ts.isTypeReferenceNode(predicate.type), `${operation.errorGuard} lost its named error predicate`);
    let errorSymbol = checker.getSymbolAtLocation(predicate.type.typeName);
    if (errorSymbol && (errorSymbol.flags & ts.SymbolFlags.Alias)) errorSymbol = checker.getAliasedSymbol(errorSymbol);
    assert.ok(errorSymbol?.declarations?.some((node) => ts.isTypeAliasDeclaration(node) && node.name.text === operation.errorType && node.getSourceFile() === operationSource), `${operation.errorGuard} compiler error binding drifted`);
    if (resolve(guardSignature.type.targetType?.reflection)?.id !== error.id) {
      assert.deepEqual(nativeShape(guardSignature.type.targetType), nativeShape(error.type), `${operation.errorGuard} native error shape drifted`);
    }
    assert.ok(commentText(guardArgs[0].comment?.summary), `Missing ${operation.errorGuard} @param documentation`);
    assert.ok((guardSignature.comment?.blockTags ?? []).some((entry) => entry.tag === "@returns" && commentText(entry.content)), `Missing @returns for ${operation.errorGuard}`);
    const successResponses = operation.responses.filter((response) => response.status >= 200 && response.status < 300);
    const errorResponses = operation.responses.filter((response) => response.status < 200 || response.status >= 300);
    const responseTuples = (type, apiName, aliasName) => {
      if (type.type === "intrinsic" && type.name === "never") return [];
      const members = type.type === "union" ? type.types : [type];
      return members.map((member) => {
        assert.equal(member.type, "reference", `${aliasName} response branch is not a documented ${apiName} reference`);
        const api = resolve(member.reflection);
        assert.equal(api?.name, apiName, `${aliasName} response branch targets ${api?.name ?? "no reflection"}, not ${apiName}`);
        assert.equal(member.typeArguments?.length, 3, `${aliasName} response branch lost generic bindings`);
        const [model, status, media] = member.typeArguments;
        if (model?.type === "intrinsic" && model.name === "undefined") {
          assert.equal(status?.type, "literal", `${aliasName} bodyless status is not literal`);
          assert.ok(media?.type === "intrinsic" && media.name === "null" || media?.type === "literal" && media.value === null,
            `${aliasName} bodyless response has a media binding`);
          assert.ok(operation.method === "HEAD" || status.value >= 100 && status.value < 200 || [204, 205, 304].includes(status.value),
            `${aliasName} erased a response body without HTTP body suppression`);
          return `@undefined\u0000${status.value}\u0000null`;
        }
        const modelReflection = resolve(model?.reflection);
        assert.ok(modelReflection && modelReflections.get(modelReflection.name)?.id === modelReflection.id,
          `${aliasName} response branch does not reference a planned model`);
        assert.equal(status?.type, "literal", `${aliasName} response status is not literal`);
        assert.equal(media?.type, "literal", `${aliasName} response media is not literal`);
        return `${modelReflection.name}\u0000${status.value}\u0000${media.value}`;
      }).sort();
    };
    const plannedTuples = (responses) => responses.map((response) => `${response.model ?? (response.nativeType === "undefined" ? "@undefined" : "@unbound")}\u0000${response.status}\u0000${response.mediaType}`).sort();
    assert.deepEqual(responseTuples(successType, "ApiResponse", operation.successType), plannedTuples(successResponses),
      `${operation.successType} branches do not exactly match planned responses`);
    assert.deepEqual(responseTuples(errorType, "DeclaredApiError", operation.errorType), plannedTuples(errorResponses),
      `${operation.errorType} branches do not exactly match planned responses`);
    const page = native(fn, operation.export);
    const source = `${operation.source.document}#${operation.source.pointer}`;
    assert.ok(page.page.text.includes(normalized(source)), `Missing or changed operation origin for ${operation.export}`);
    if (operation.hasSourceDescription) assert.ok(page.page.text.includes(normalized(operation.descriptionText)), `Source prose was reinterpreted for ${operation.export}`);
    operationCoverage.push({ operationId: operation.operationId, name: operation.export, inputType: operation.inputType,
      successType: operation.successType, errorType: operation.errorType, errorGuard: operation.errorGuard,
      source: operation.source, security: operation.security, url: page.url, parameters: operation.parameters, responses: operation.responses, documented: true });
  }
}
const supportDeclarations = project.getReflectionsByKind(ReflectionKind.TypeAlias | ReflectionKind.Class | ReflectionKind.Interface);
for (const name of manifest.supportSymbols) {
  assert.equal(supportDeclarations.filter((declaration) => declaration.name === name).length, 1, `Missing support type ${name}`);
}
const report = {
  format: "suspect-typescript-docs-coverage-v1", complete: true,
  releaseReady: manifest.releaseReady, codecsImplemented,
  toolchain: { node: process.versions.node, ...own.devDependencies,
    packageLockSha256: createHash("sha256").update(lockBytes).digest("hex") },
  symbols: coverage, ...(codecsImplemented ? { codecs: codecCoverage } : {}), ...(httpImplemented ? { operations: operationCoverage, httpImplemented: true } : {}),
  htmlDocuments: documents.size, checkedLocalLinks: localLinks,
  missingSourceDescriptions: coverage.filter((symbol) => !symbol.hasSourceDescription).map((symbol) => symbol.name),
  findings: manifest.findings,
};
await writeFile(path.join(output, "coverage.json"), `${JSON.stringify(report, null, 2)}\n`);
console.log(`Documented ${coverage.length} canonical model symbols${codecsImplemented ? ` and ${codecCoverage.length} validated codecs` : ""}${httpImplemented ? ` and ${operationCoverage.length} HTTP operations` : ""} in ${documents.size} HTML pages; ${localLinks} local links checked.`);
