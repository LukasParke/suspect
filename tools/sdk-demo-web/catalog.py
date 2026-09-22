"""Presentation metadata; code excerpts are read verbatim from the sealed guide."""
from __future__ import annotations

import re
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
PREPARED_ROOT = REPO / "target/sdk-demo-live-20260911-01/environment-01/candidate-02"
REFERENCE = REPO / "LIVE-ENV-DEMO-README.md"
SOURCE_ROOT = REPO / "examples/sdk-demo-live/environment"
LANGUAGES = (
    "typescript", "python", "go", "rust", "swift", "java", "csharp",
    "kotlin", "ruby", "php", "dart", "cpp",
)
CONSUMERS = (*LANGUAGES, "javascript")
OPERATIONS = {"key": "/key"}
SOURCE_SERVER = "https://openrouter.ai/api/v1"

# Assumptions are visible beside the excerpt, rather than hidden in copyable code.
META = {
    "typescript": {
        "name": "TypeScript", "badge": "TS", "file": "main.ts", "fence": "ts",
        "group": "web", "flavor": "Strict types · ESM",
        "assumption": "ESM with top-level await. The full program adds an AbortSignal deadline.",
    },
    "python": {
        "name": "Python", "badge": "Py", "file": "main.py", "fence": "python",
        "group": "web", "flavor": "Typed models · Context manager",
        "assumption": "Installed wheel. The context manager owns cleanup; usage keeps its exact number token.",
    },
    "go": {
        "name": "Go", "badge": "Go", "file": "main.go", "fence": "go",
        "group": "systems", "flavor": "Native context · Typed responses",
        "assumption": 'Imports fmt and sdk "github.com/openrouter/sdk-go". Inside a function returning error; ctx is app-owned.',
    },
    "rust": {
        "name": "Rust", "badge": "Rs", "file": "main.rs", "fence": "rust",
        "group": "systems", "flavor": "Async · Result",
        "assumption": "Inside an async function returning Result. The application owns Tokio; reqwest-rustls is enabled.",
    },
    "swift": {
        "name": "Swift", "badge": "Sw", "file": "Main.swift", "fence": "swift",
        "group": "platform", "flavor": "SwiftPM · Async/await",
        "assumption": "Imports module OpenRouter from OpenRouterSDK. Inside an async, throwing entry point.",
    },
    "java": {
        "name": "Java", "badge": "Jv", "file": "Main.java", "fence": "java",
        "group": "platform", "flavor": "Maven · AutoCloseable",
        "assumption": "Imports ai.openrouter.sdk.*. Inside an application method; try-with-resources owns cleanup.",
    },
    "csharp": {
        "name": "C#", "badge": "C#", "file": "Program.cs", "fence": "csharp",
        "group": "platform", "flavor": "NuGet · Task / CancellationToken",
        "assumption": "Using OpenRouter in an async entry point. The app owns cancellationToken; FromEnvironment loads the key.",
    },
    "kotlin": {
        "name": "Kotlin", "badge": "Kt", "file": "Smoke.kt", "fence": "kotlin",
        "group": "platform", "flavor": "Maven · Suspending calls",
        "assumption": "Imports ai.openrouter.kotlin.*. Inside a suspending function with an application-owned coroutine deadline.",
    },
    "ruby": {
        "name": "Ruby", "badge": "Rb", "file": "main.rb", "fence": "ruby",
        "group": "web", "flavor": "Gem · Block-scoped client",
        "assumption": "Requires 'openrouter' from the installed gem. The block closes the client automatically.",
    },
    "php": {
        "name": "PHP", "badge": "php", "file": "main.php", "fence": "php",
        "group": "web", "flavor": "Composer · Native Curl",
        "assumption": "PHP source with Composer's vendor/autoload.php loaded. Native Curl transport and typed errors.",
    },
    "dart": {
        "name": "Dart", "badge": "Da", "file": "main.dart", "fence": "dart",
        "group": "systems", "flavor": "Native executable · IO transport",
        "assumption": "Inside an async entry point. Credentials are omitted; the full program closes the client in finally.",
    },
    "cpp": {
        "name": "C++", "badge": "C++", "file": "main.cpp", "fence": "cpp",
        "group": "systems", "flavor": "CMake · Result / RAII",
        "assumption": "Includes <openrouter/sdk.hpp> and <iostream>; namespace openrouter. failed(...) is the app's error handler.",
    },
    "javascript": {
        "name": "JavaScript", "badge": "JS", "file": "main.mjs", "fence": "js",
        "group": "bonus", "flavor": "Same ESM package · Plain JavaScript",
        "assumption": "ESM with top-level await. A separate JavaScript consumer of the installed TypeScript SDK.",
    },
}


def excerpts() -> dict[str, str]:
    fences = dict(re.findall(r"```(\w+)\n(.*?)\n```", REFERENCE.read_text(), re.S))
    result = {language: fences[META[language]["fence"]] for language in CONSUMERS}
    if any(not 3 <= len(code.splitlines()) <= 4 for code in result.values()):
        raise ValueError("The accepted short examples have changed")
    return result


def source_paths() -> dict[str, Path]:
    return {language: SOURCE_ROOT / language / META[language]["file"] for language in CONSUMERS}


def document_paths() -> dict[str, Path]:
    return {
        language: PREPARED_ROOT / "packages" / ("typescript" if language == "javascript" else language) / "README.md"
        for language in CONSUMERS
    }
