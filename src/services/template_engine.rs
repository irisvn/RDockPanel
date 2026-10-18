//! Runs an EasyPanel template's generator (`index.ts`): TypeScript with its
//! types stripped (oxc), evaluated in QuickJS with nothing but what a
//! template may import:
//! - `~templates-utils`: `randomString`, `randomPassword`, `bcryptHash`;
//! - `./meta`: the template's `meta.yaml`;
//! - `crypto`: `randomBytes`.
//!
//! No file system, no network, a memory, stack and time budget. Its
//! `generate(input)` answers `{services: [{type, data}]}`.
//!
//! **Isolated.** The panel runs a template in a child process of its own
//! binary ([`run_isolated`]): compiling (oxc) is bounded by the source's
//! size, running (QuickJS) by its budget, and the whole by the child's CPU
//! and memory limits and a wall-clock kill. A template that crashes or
//! hangs takes its child down, never the panel.

use std::path::Path;
use std::time::{Duration, Instant};

use rquickjs::loader::{BuiltinLoader, BuiltinResolver};
use rquickjs::{CatchResultExt, Context, Function, Module, Runtime, Value as JsValue};
use serde_json::Value;

use crate::services::database_service::random_string;

/// How long a generator may run.
const TIME: Duration = Duration::from_secs(3);
/// The largest generator or `meta.yaml` (EasyPanel's are a few KB).
pub const MAX_SOURCE: usize = 256 * 1024;
/// How many `bcryptHash` calls a run may make (each costs a few ms).
const MAX_HASHES: usize = 16;
/// The child process's argument (`main.rs`).
pub const CHILD_ARG: &str = "__rdockpanel-run-template";
/// How long the child may live, compile and run included.
const CHILD_TIME: Duration = Duration::from_secs(15);
/// How much memory it may take.
const MEMORY: usize = 64 * 1024 * 1024;

/// `source` (TypeScript) as JavaScript, types stripped.
pub fn compile(source: &str) -> Result<String, String> {
    if source.len() > MAX_SOURCE {
        return Err("the template is too large".to_owned());
    }
    use oxc::allocator::Allocator;
    use oxc::codegen::Codegen;
    use oxc::parser::Parser;
    use oxc::semantic::SemanticBuilder;
    use oxc::span::SourceType;
    use oxc::transformer::{TransformOptions, Transformer};

    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::ts()).parse();
    if let Some(error) = parsed.diagnostics.first() {
        return Err(format!("the template does not parse: {error}"));
    }
    let mut program = parsed.program;
    let scoping = SemanticBuilder::new()
        .build(&program)
        .semantic
        .into_scoping();
    let options = TransformOptions::default();
    let done = Transformer::new(&allocator, Path::new("index.ts"), &options)
        .build_with_scoping(scoping, &mut program);
    if let Some(error) = done.diagnostics.first() {
        return Err(format!("the template does not compile: {error}"));
    }
    Ok(Codegen::new().build(&program).code)
}

/// `~templates-utils` as EasyPanel's (`utils/index.ts`), on the panel's
/// random source.
const UTILS: &str = r#"
export function randomString(length = 10) { return globalThis.__rdpRandom(length); }
export function randomPassword() { return randomString(20); }
export function bcryptHash(text) { return globalThis.__rdpBcrypt(String(text)); }
"#;

/// The part of Node's `crypto` and `Buffer` templates use, as functions
/// (shared by the `crypto` module and the globals).
const CRYPTO_CORE: &str = r#"
function __rdpHexToBytes(hex) { const out = []; for (let i = 0; i < hex.length; i += 2) out.push(parseInt(hex.slice(i, i + 2), 16)); return out; }
const __rdpB64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
function __rdpBase64(bytes) { let out = ""; for (let i = 0; i < bytes.length; i += 3) { const n = (bytes[i] << 16) | ((bytes[i + 1] ?? 0) << 8) | (bytes[i + 2] ?? 0); out += __rdpB64[(n >> 18) & 63] + __rdpB64[(n >> 12) & 63] + (i + 1 < bytes.length ? __rdpB64[(n >> 6) & 63] : "=") + (i + 2 < bytes.length ? __rdpB64[n & 63] : "="); } return out; }
function __rdpEncode(bytes, encoding) {
  if (encoding === "base64") return __rdpBase64(bytes);
  if (encoding === "base64url") return __rdpBase64(bytes).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  if (encoding === "hex") return bytes.map((b) => b.toString(16).padStart(2, "0")).join("");
  return decodeURIComponent(escape(String.fromCharCode(...bytes)));
}
function __rdpBytes(bytes) { return { length: bytes.length, toString(encoding) { return __rdpEncode(bytes, encoding ?? "hex"); } }; }
function __rdpRandomBytes(size) { return __rdpBytes(__rdpHexToBytes(globalThis.__rdpRandomHex(size))); }
function __rdpRandomUUID() { const h = globalThis.__rdpRandomHex(16); return `${h.slice(0, 8)}-${h.slice(8, 12)}-4${h.slice(13, 16)}-a${h.slice(17, 20)}-${h.slice(20, 32)}`; }
"#;

/// `crypto` (and `node:crypto`), for `import`.
fn crypto_module() -> String {
    format!(
        "{CRYPTO_CORE}\nexport const randomBytes = __rdpRandomBytes;\nexport const randomUUID = __rdpRandomUUID;\nexport default {{ randomBytes, randomUUID }};\n"
    )
}

/// What CommonJS-era templates reach without importing: `require("crypto")`
/// and `Buffer.from(text).toString(encoding)`.
fn prelude() -> String {
    format!(
        r#"{CRYPTO_CORE}
globalThis.require = (name) => {{
  if (name === "crypto" || name === "node:crypto") return {{ randomBytes: __rdpRandomBytes, randomUUID: __rdpRandomUUID }};
  throw new Error(`a template cannot require ${{name}}`);
}};
globalThis.Buffer = {{
  from(value, encoding) {{
    if (typeof value !== "string") return __rdpBytes(Array.from(value));
    if (encoding === "hex") return __rdpBytes(__rdpHexToBytes(value));
    const utf8 = unescape(encodeURIComponent(value));
    return __rdpBytes(Array.from(utf8, (c) => c.charCodeAt(0)));
  }},
}};
"#
    )
}

/// Lowercase letters and digits, as EasyPanel's `randomString`.
fn random_lower(length: usize) -> String {
    random_string(length.clamp(1, 512) * 2)
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .take(length.clamp(1, 512))
        .collect()
}

fn random_hex(size: usize) -> String {
    let mut bytes = vec![0u8; size.clamp(1, 1024)];
    getrandom::fill(&mut bytes).expect("the system's random source");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Runs `generate(input)` of the compiled template `js` with its `meta`.
pub fn generate(js: &str, meta: &Value, input: &Value) -> Result<Value, String> {
    let fail = |e: rquickjs::Error| format!("the template's runtime: {e}");
    let runtime = Runtime::new().map_err(fail)?;
    runtime.set_memory_limit(MEMORY);
    runtime.set_max_stack_size(1024 * 1024);
    let deadline = Instant::now() + TIME;
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline)));
    let meta_module = format!("export const meta = {meta};\nexport default meta;\n");
    runtime.set_loader(
        BuiltinResolver::default()
            .with_module("~templates-utils")
            .with_module("template/meta")
            .with_module("crypto")
            .with_module("node:crypto"),
        BuiltinLoader::default()
            .with_module("~templates-utils", UTILS)
            .with_module("template/meta", meta_module)
            .with_module("crypto", crypto_module())
            .with_module("node:crypto", crypto_module()),
    );
    let context = Context::full(&runtime).map_err(fail)?;
    context.with(|ctx| {
        let globals = ctx.globals();
        globals
            .set(
                "__rdpRandom",
                Function::new(ctx.clone(), random_lower).map_err(fail)?,
            )
            .map_err(fail)?;
        globals
            .set(
                "__rdpRandomHex",
                Function::new(ctx.clone(), random_hex).map_err(fail)?,
            )
            .map_err(fail)?;
        // Hashing is native work QuickJS cannot interrupt: a few, in time.
        let hashes = std::cell::Cell::new(0usize);
        globals
            .set(
                "__rdpBcrypt",
                Function::new(
                    ctx.clone(),
                    move |text: String| -> rquickjs::Result<String> {
                        hashes.set(hashes.get() + 1);
                        if hashes.get() > MAX_HASHES || Instant::now() > deadline {
                            return Err(rquickjs::Error::new_from_js_message(
                                "string",
                                "hash",
                                "too many bcryptHash calls",
                            ));
                        }
                        let text: String = text.chars().take(72).collect();
                        Ok(bcrypt::hash(text, 10).unwrap_or_default())
                    },
                )
                .map_err(fail)?,
            )
            .map_err(fail)?;
        let run = || -> rquickjs::Result<String> {
            ctx.eval::<(), _>(prelude())?;
            let (module, promise) = Module::declare(ctx.clone(), "template/index", js)?.eval()?;
            promise.finish::<()>()?;
            let generate: Function = module.get("generate")?;
            let input = ctx.json_parse(input.to_string())?;
            let output: JsValue = generate.call((input,))?;
            let text = ctx
                .json_stringify(output)?
                .map(|s| s.to_string())
                .transpose()?
                .unwrap_or_default();
            Ok(text)
        };
        let text = run()
            .catch(&ctx)
            .map_err(|e| format!("the template failed: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("the template's output: {e}"))
    })
}

/// The child's side: reads `{code, meta, input}` on stdin, answers
/// `{"output"}` or `{"error"}` on stdout, within its limits.
pub fn child_main() -> std::process::ExitCode {
    use std::io::{Read, Write};
    limit_self();
    let mut text = String::new();
    if std::io::stdin()
        .take((MAX_SOURCE * 3) as u64)
        .read_to_string(&mut text)
        .is_err()
    {
        return std::process::ExitCode::FAILURE;
    }
    // A thread of its own, with room for deep (but bounded) nesting.
    let answer = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let request: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            let code = request["code"].as_str().unwrap_or_default();
            let js = compile(code)?;
            generate(&js, &request["meta"], &request["input"])
        })
        .ok()
        .and_then(|t| t.join().ok())
        .unwrap_or_else(|| Err("the template crashed".to_owned()));
    let reply = match answer {
        Ok(output) => serde_json::json!({ "output": output }),
        Err(error) => serde_json::json!({ "error": error }),
    };
    let _ = std::io::stdout().write_all(reply.to_string().as_bytes());
    std::process::ExitCode::SUCCESS
}

/// CPU seconds and (on Linux) address space for this child.
fn limit_self() {
    use rustix::process::{Resource, Rlimit, setrlimit};
    let _ = setrlimit(
        Resource::Cpu,
        Rlimit {
            current: Some(10),
            maximum: Some(12),
        },
    );
    #[cfg(target_os = "linux")]
    let _ = setrlimit(
        Resource::As,
        Rlimit {
            current: Some(1024 * 1024 * 1024),
            maximum: Some(1024 * 1024 * 1024),
        },
    );
}

/// Where templates run.
#[derive(Debug, Clone)]
pub enum Runner {
    /// A child process of this binary (the panel).
    Child(std::path::PathBuf),
    /// In this process (tests: their binary is not the panel's).
    InProcess,
}

impl Runner {
    /// The panel's own binary as the child.
    pub fn child() -> Self {
        std::env::current_exe().map_or(Self::InProcess, Self::Child)
    }
}

/// Compiles and runs template `code` with `meta` and `input`, as `runner` says.
pub async fn run_isolated(
    runner: &Runner,
    code: &str,
    meta: &Value,
    input: &Value,
) -> Result<Value, String> {
    if code.len() > MAX_SOURCE {
        return Err("the template is too large".to_owned());
    }
    let path = match runner {
        Runner::InProcess => {
            let (code, meta, input) = (code.to_owned(), meta.clone(), input.clone());
            return tokio::task::spawn_blocking(move || generate(&compile(&code)?, &meta, &input))
                .await
                .map_err(|e| e.to_string())?;
        }
        Runner::Child(path) => path,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut child = tokio::process::Command::new(path)
        .arg(CHILD_ARG)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .env_clear()
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("the template could not start: {e}"))?;
    let request = serde_json::json!({ "code": code, "meta": meta, "input": input }).to_string();
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let work = async move {
        stdin
            .write_all(request.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        drop(stdin);
        let mut reply = Vec::new();
        (&mut stdout)
            .take(4 * 1024 * 1024)
            .read_to_end(&mut reply)
            .await
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(reply)
    };
    let reply = match tokio::time::timeout(CHILD_TIME, work).await {
        Ok(reply) => reply?,
        Err(_) => {
            let _ = child.kill().await;
            return Err("the template took too long".to_owned());
        }
    };
    let _ = child.wait().await;
    let reply: Value =
        serde_json::from_slice(&reply).map_err(|_| "the template crashed".to_owned())?;
    match reply.get("output") {
        Some(output) => Ok(output.clone()),
        None => Err(reply["error"]
            .as_str()
            .unwrap_or("the template failed")
            .to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A template as EasyPanel's repository writes them.
    const GHOST: &str = r#"
import { Output, randomPassword, randomString, Services } from "~templates-utils";
import { Input } from "./meta";

export function generate(input: Input): Output {
  const services: Services = [];
  const databasePassword: string = randomPassword();
  services.push({
    type: "app",
    data: {
      serviceName: input.appServiceName,
      source: { type: "image", image: input.appServiceImage },
      domains: [{ host: "$(EASYPANEL_DOMAIN)", port: 2368 }],
      env: [
        `url=https://$(PRIMARY_DOMAIN)`,
        `database__connection__host=$(PROJECT_NAME)_${input.databaseServiceName}`,
        `database__connection__password=${databasePassword}`,
        `token=${randomString(8)}`,
      ].join("\n"),
    },
  });
  services.push({ type: "mysql", data: { serviceName: input.databaseServiceName, password: databasePassword } });
  return { services };
}
"#;

    #[test]
    fn a_template_generates_its_services() {
        let js = compile(GHOST).unwrap();
        assert!(!js.contains(": string"), "{js}");
        let meta = json!({"name": "Ghost"});
        let out = generate(&js, &meta, &json!({"appServiceName": "ghost", "appServiceImage": "ghost:6", "databaseServiceName": "ghost-db"})).unwrap();
        let services = out["services"].as_array().unwrap();
        assert_eq!(services.len(), 2);
        assert_eq!(services[0]["data"]["serviceName"], "ghost");
        let env = services[0]["data"]["env"].as_str().unwrap();
        let password = services[1]["data"]["password"].as_str().unwrap();
        assert_eq!(password.len(), 20);
        assert!(
            password
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
        assert!(env.contains(&format!("password={password}")));
        assert!(env.contains("host=$(PROJECT_NAME)_ghost-db"));
    }

    #[test]
    fn templates_get_crypto_and_bcrypt_only() {
        let js = compile(
            r#"import { randomBytes } from "crypto";
               import { bcryptHash } from "~templates-utils";
               export function generate(input: any) { return { secret: randomBytes(16).toString("hex"), hash: bcryptHash("x"), b64: randomBytes(3).toString("base64") }; }"#,
        )
        .unwrap();
        let out = generate(&js, &json!({}), &json!({})).unwrap();
        assert_eq!(out["secret"].as_str().unwrap().len(), 32);
        assert!(out["hash"].as_str().unwrap().starts_with("$2"));
        assert_eq!(out["b64"].as_str().unwrap().len(), 4);
        // CommonJS-era: require("crypto"), Buffer.
        let js = compile(
            r#"export function generate() { const c = require("crypto"); return { s: c.randomBytes(4).toString("hex"), b: Buffer.from("hi").toString("base64") }; }"#,
        )
        .unwrap();
        let out = generate(&js, &json!({}), &json!({})).unwrap();
        assert_eq!(out["s"].as_str().unwrap().len(), 8);
        assert_eq!(out["b"], "aGk=");
        // Anything else cannot be imported.
        let js =
            compile(r#"import fs from "fs"; export function generate() { return fs; }"#).unwrap();
        assert!(generate(&js, &json!({}), &json!({})).is_err());
    }

    #[test]
    fn a_runaway_template_is_stopped() {
        let js = compile("export function generate() { while (true) {} }").unwrap();
        let started = Instant::now();
        assert!(generate(&js, &json!({}), &json!({})).is_err());
        assert!(started.elapsed() < Duration::from_secs(10));
        let js = compile("export function generate() { const a = []; while (true) a.push(new Array(1e6).fill(1)); }").unwrap();
        assert!(generate(&js, &json!({}), &json!({})).is_err());
    }
}
