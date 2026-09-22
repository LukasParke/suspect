//! Source-linked Terraform/native documentation and executable HCL variables.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{emit::q, planning::*, *};

pub(super) fn source(id: &SourceId) -> Value {
    json!({"document":id.document().as_str(), "pointer":id.pointer()})
}

pub(super) fn artifacts(plan: &ProviderPlan) -> Vec<OutFile> {
    let config = &plan.config;
    let mut files = Vec::new();
    let mut calls = Vec::new();
    for r in &plan.resources {
        for (phase, call) in [
            ("create", &r.create),
            ("read", &r.read),
            ("update", &r.update),
            ("delete", &r.delete),
        ] {
            calls.push(binding(&r.name, "resource", phase, call));
        }
        let mapping = &plan.profile.resources[&r.name];
        files.push(OutFile {
            path: format!("terraform/docs/resources/{}.md", r.name),
            content: page(
                plan,
                &r.name,
                &mapping.description,
                &mapping.attributes,
                Some(&mapping.identity.attribute),
            ),
        });
        files.push(OutFile {
            path: format!("terraform/examples/resources/{}/main.tf", r.name),
            content: hcl(plan, &r.name, &mapping.attributes, false),
        });
    }
    for d in &plan.data_sources {
        calls.push(binding(&d.name, "data_source", "read", &d.read));
        let mapping = &plan.profile.data_sources[&d.name];
        files.push(OutFile {
            path: format!("terraform/docs/data-sources/{}.md", d.name),
            content: page(
                plan,
                &d.name,
                &mapping.description,
                &mapping.attributes,
                None,
            ),
        });
        files.push(OutFile {
            path: format!("terraform/examples/data-sources/{}/main.tf", d.name),
            content: hcl(plan, &d.name, &mapping.attributes, true),
        });
    }
    let sdk_files = plan.sdk_files.iter().map(|f| json!({"path":f.path,"sha256":format!("{:x}",Sha256::digest(f.content.as_bytes()))})).collect::<Vec<_>>();
    let documents = plan.sdk.contract().documents().map(|(uri, raw)| json!({"document":uri.as_str(), "canonical_json_sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(raw).unwrap()))})).collect::<Vec<_>>();
    let manifest = json!({"format":"suspect.terraform.artifact.v1", "lifecycle_profile":PROFILE, "configuration":config, "mapping":plan.profile, "sources":documents, "sdk_files":sdk_files, "native_bindings":calls, "installation":"Pinned SDK module dependency. Local proxy/mirror acceptance is not publishing.", "policy":{"unknown":"reject unresolved inputs at apply; never construct API defaults", "null":"per-input reject/omit/send_null", "read_absence_or_null":"Terraform null", "refresh":"authoritative read; only declared missing errors remove resource state", "partial":"mapped response state plus error; other errors retain prior state", "retry":"none", "polling":"none"}});
    files.push(OutFile {
        path: "terraform/source-bindings.json".into(),
        content: format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    });
    files.push(OutFile {
        path: "terraform/lifecycle-mapping.json".into(),
        content: format!("{}\n", serde_json::to_string_pretty(&plan.profile).unwrap()),
    });
    let mut readme = format!(
        "# Terraform provider `{}`\n\nGenerated with `{PROFILE}`. Provider `{}` version `{}`; Go `{}` / `{}`; Terraform `={}`; Plugin Framework `={}`.\n\nEvery API call is a typed operation on the generated SDK module `{} v{}`. Authentication, HTTP, codecs, validation, and errors belong to that dependency.\n\n## Build and local installation\n\nMake the exact SDK version available through your Go module proxy, then:\n\n```sh\ngo mod tidy\ngo test ./...\ngo build -o terraform-provider-{}_v{} .\ngo doc -all ./provider\n```\n\nInstall the binary in a Terraform filesystem mirror at `<mirror>/{}/{}/<os>_<arch>/`. Select the mirror with a Terraform CLI `provider_installation {{ filesystem_mirror {{ path = \"<absolute mirror>\" }} }}` block. Examples pin the provider source/version and Terraform version. Local mirror installation does not publish the provider or SDK.\n\n## Executable examples and native schema\n\nEach directory under `examples/resources` and `examples/data-sources` is an independent Terraform consumer. Set `TF_VAR_endpoint` and the declared input variables, including `TF_VAR_token` for bearer authentication. Optional inputs default to Terraform null; this is not an API default. Then run:\n\n```sh\nterraform init\nterraform validate\nterraform plan -out=tfplan\nterraform apply tfplan\nterraform providers schema -json\nterraform refresh\nterraform destroy\n```\n\nThe Markdown schema tables below `docs/` and the native `provider` Go comments are generated from the same admitted plan. `terraform providers schema -json` exposes the real Plugin Framework schema. `source-bindings.json` retains exact native operation/input/status/field names, physical source identities and the canonical SDK artifact hashes.\n\n## Lifecycle policy\n\n- Operations and input/state paths are explicit; no endpoint-name CRUD inference.\n- Unknown inputs are preserved during planning and rejected if unresolved at apply. Null follows each binding's explicit omission/null/reject policy.\n- Create/update retain the mapped SDK response. Known planned values must agree; disagreement is an error with authoritative state retained. Refresh reads authoritative state and reports drift.\n- A declared missing read response removes the resource; a missing data-source result is a diagnostic. A declared missing delete result is success.\n- Exact nonempty opaque-string IDs import without normalization. The next read fills remote attributes. Terraform-only version triggers are null on import.\n- Sensitive attributes are redacted in CLI output but remain in state. Write-only attributes are read from configuration, sent through the SDK and never saved in state. Update only sends them when the explicitly mapped trigger changes; provide a non-null secret on a trigger change.\n- A mapped partial error saves its returned remote fields, returns an error and lets Terraform taint failed creates. Failed updates preserve prior trigger values. Other failures retain prior state, or no state on create; uncertain remote effects require refresh/import.\n- There is one SDK call per lifecycle invocation, with the original Terraform context. No provider retry, polling, detached work or raw HTTP client exists.\n",
        config.provider_name,
        config.provider_address,
        config.version,
        config.go_version,
        config.go_toolchain,
        config.terraform_version,
        config.framework_version,
        config.sdk.module_path,
        config.sdk.version,
        config.provider_name,
        config.version,
        config.provider_address,
        config.version
    );
    readme.push_str("\nSee `lifecycle-mapping.json` for the exact admitted lifecycle and `source-bindings.json` for native/source provenance.\n");
    files.push(OutFile {
        path: "terraform/README.md".into(),
        content: readme.clone(),
    });
    files.push(OutFile {
        path: "terraform/docs/index.md".into(),
        content: readme,
    });
    files
}

fn binding(name: &str, kind: &str, phase: &str, call: &BoundCall) -> Value {
    let response = |r: &BoundResponse| json!({"native_type":r.response.type_name, "native_payload":r.response.native_type, "status":r.response.wire().status_key(), "source":source(r.response.wire().source().use_site().source()), "outputs":r.outputs.iter().map(|o|json!({"attribute":o.attribute,"native_field":o.field.name,"native_value_type":o.field.scalar.value_type,"source":source(&o.field.source)})).collect::<Vec<_>>()});
    json!({"type":name,"kind":kind,"phase":phase,"operation_id":call.operation.operation_id,"native_method":call.operation.method_name,"native_input":call.operation.input_type,"native_constructor":call.operation.input_constructor,"source":source(&call.operation.source),"inputs":call.inputs.iter().map(|i| json!({"mapping":i.mapping,"source":source(&i.source),"native_field":i.member,"native_value_type":i.scalar.value_type,"body":i.body})).collect::<Vec<_>>(),"success":response(&call.success),"partial":call.partial.iter().map(response).collect::<Vec<_>>(),"missing":call.missing.iter().map(|r|json!({"status":r.wire().status_key(),"native_type":r.type_name,"source":source(r.wire().source().use_site().source())})).collect::<Vec<_>>()})
}

fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('|', "&#124;")
        .replace(['\r', '\n'], " ")
}

fn page(
    plan: &ProviderPlan,
    name: &str,
    description: &str,
    attrs: &std::collections::BTreeMap<String, Attribute>,
    identity: Option<&str>,
) -> String {
    let mut text = format!(
        "# {}_{} ({})\n\n{}\n\n## Schema\n\n| Attribute | Type | Mode | Sensitive | Write-only | Replacement | Description |\n| --- | --- | --- | --- | --- | --- | --- |\n",
        plan.config.provider_name,
        name,
        if identity.is_some() {
            "Resource"
        } else {
            "Data Source"
        },
        escaped(description)
    );
    for (name, a) in attrs {
        text.push_str(&format!(
            "| `{name}` | `{}` | {} | {} | {} | {} | {} |\n",
            a.r#type.hcl(),
            serde_json::to_value(a.mode).unwrap().as_str().unwrap(),
            a.sensitive,
            a.write_only,
            a.requires_replace,
            escaped(&a.description)
        ));
    }
    text.push_str("\n## Example\n\n```hcl\n");
    text.push_str(&hcl(plan, name, attrs, identity.is_none()));
    text.push_str("```\n");
    if let Some(identity) = identity {
        text.push_str(&format!("\n## Import\n\nThe exact nonempty opaque string is stored in `{identity}`. Import calls the mapped SDK read operation to fill state.\n\n```sh\nterraform import {}_{}.example 'REMOTE_ID'\n```\n", plan.config.provider_name, name));
    }
    text
}

fn hcl(
    plan: &ProviderPlan,
    name: &str,
    attrs: &std::collections::BTreeMap<String, Attribute>,
    data: bool,
) -> String {
    let config = &plan.config;
    let mut hcl = format!(
        "terraform {{\n  required_version = {}\n  required_providers {{\n    {} = {{\n      source = {}\n      version = {}\n    }}\n  }}\n}}\n\nvariable \"endpoint\" {{\n  type = string\n}}\n",
        q(&format!("= {}", config.terraform_version)),
        config.provider_name,
        q(&config.provider_address),
        q(&format!("= {}", config.version))
    );
    if plan.credential.is_some() {
        hcl.push_str(
            "\nvariable \"token\" {\n  type = string\n  sensitive = true\n  ephemeral = true\n}\n",
        );
    }
    hcl.push_str(&format!(
        "\nprovider {} {{\n  endpoint = var.endpoint\n{}\n}}\n",
        q(&config.provider_name),
        if plan.credential.is_some() {
            "  token = var.token"
        } else {
            ""
        }
    ));
    for (name, a) in attrs.iter().filter(|(_, a)| a.mode.configured()) {
        hcl.push_str(&format!(
            "\nvariable {} {{\n  type = {}\n",
            q(&format!("input_{name}")),
            a.r#type.hcl()
        ));
        if a.mode != AttributeMode::Required {
            hcl.push_str("  default = null\n");
        }
        if a.sensitive {
            hcl.push_str("  sensitive = true\n");
        }
        if a.write_only {
            hcl.push_str("  ephemeral = true\n");
        }
        hcl.push_str("}\n");
    }
    hcl.push_str(&format!(
        "\n{} {} \"example\" {{\n",
        if data { "data" } else { "resource" },
        q(&format!("{}_{name}", config.provider_name))
    ));
    for (name, _) in attrs.iter().filter(|(_, a)| a.mode.configured()) {
        hcl.push_str(&format!("  {name} = var.input_{name}\n"));
    }
    hcl.push_str("}\n");
    hcl
}
