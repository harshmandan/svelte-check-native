//! The compiler's validation of its compile options
//! (`validate-options.js`), applied to the `compilerOptions` of the
//! project's Svelte config.
//!
//! svelte-check compiles every component with `{ dev: true,
//! ...compilerOptions, generate: false, filename }`. The compiler
//! validates those options before it parses anything:
//!
//! - an unknown key, a removed option, or a value of the wrong type
//!   throws, so every component reports that one error (at the start of
//!   the file) and nothing else the compiler would say;
//! - the `css` and `customElement` options are only read during the
//!   analysis, so a wrong value there throws at that point instead;
//! - deprecated and removed-but-ignored options warn through a
//!   process-wide "warn once" set, so only the first component
//!   svelte-check compiles shows those warnings.
//!
//! The config is read statically; a value that is not a literal stops
//! the check at the point it would need that value, reporting only what
//! was already certain.

use crate::codes::Code;
use crate::messages;

/// A compile option's value as far as a static read of the config can
/// tell.
#[derive(Debug, Clone, PartialEq)]
pub enum OptionValue {
    Bool(bool),
    Str(String),
    Num(f64),
    Null,
    Undefined,
    Array,
    Function,
    Object(Vec<(String, OptionValue)>),
    /// Anything whose value is only known when the config runs.
    Unknown,
}

/// What validating a config's compile options yields.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CompileOptionsCheck {
    /// The error validation throws before parsing.
    pub error: Option<(Code, String)>,
    /// The error reading `css` / `customElement` throws during the
    /// analysis: `(option, code, message)`.
    pub late_error: Option<(LateOption, Code, String)>,
    /// The warnings validation emits (once per process).
    pub warnings: Vec<(Code, String)>,
}

/// An option the compiler validates only when the analysis reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LateOption {
    CustomElement,
    Css,
}

impl CompileOptionsCheck {
    /// Nothing to report.
    pub fn is_empty(&self) -> bool {
        self.error.is_none() && self.late_error.is_none() && self.warnings.is_empty()
    }
}

/// A validation step could not decide: the check stops there.
struct Undecided;

type Step = Result<(), Undecided>;

/// Validate the user's `compilerOptions` (in source order) the way the
/// compiler validates `{ dev: true, ...compilerOptions, generate: false,
/// filename }`.
pub fn check_compile_options(user: &[(String, OptionValue)]) -> CompileOptionsCheck {
    let mut check = CompileOptionsCheck::default();
    let _ = run(user, &mut check);
    check
}

fn run(user: &[(String, OptionValue)], check: &mut CompileOptionsCheck) -> Step {
    // `for (const key in input)`: unknown keys, in key order.
    for (key, _) in user {
        if !is_component_option(key) {
            return fail(
                check,
                Code::options_unrecognised,
                messages::options_unrecognised(key),
            );
        }
    }
    let get = |key: &str| -> &OptionValue {
        user.iter()
            .rev()
            .find(|(k, _)| k == key)
            .map_or(&OptionValue::Undefined, |(_, v)| v)
    };
    // The merged options fix `generate` and `filename`; `dev` defaults
    // to true.
    string(check, "rootDir", get("rootDir"))?;
    boolean(check, "dev", get("dev"))?;
    fun(check, "warningFilter", get("warningFilter"))?;
    let value = group(check, "experimental", get("experimental"), "async")?;
    boolean(check, "experimental.async", &value)?;
    deprecated(
        check,
        get("accessors"),
        Code::options_deprecated_accessors,
        messages::options_deprecated_accessors(),
    )?;
    boolean(check, "accessors", get("accessors"))?;
    let css = get("css").clone();
    fun(check, "cssHash", get("cssHash"))?;
    string(check, "cssOutputFilename", get("cssOutputFilename"))?;
    let custom_element = get("customElement").clone();
    boolean(check, "discloseVersion", get("discloseVersion"))?;
    deprecated(
        check,
        get("immutable"),
        Code::options_deprecated_immutable,
        messages::options_deprecated_immutable(),
    )?;
    boolean(check, "immutable", get("immutable"))?;
    removed(
        check,
        get("legacy"),
        "The legacy option has been removed. If you are using this because of legacy.componentApi, use compatibility.componentApi instead",
    )?;
    let value = group(check, "compatibility", get("compatibility"), "componentApi")?;
    list(
        check,
        "compatibility.componentApi",
        &value,
        &[ListItem::Num(4.0), ListItem::Num(5.0)],
    )?;
    deprecated(
        check,
        get("loopGuardTimeout"),
        Code::options_removed_loop_guard_timeout,
        messages::options_removed_loop_guard_timeout(),
    )?;
    string(check, "name", get("name"))?;
    list(
        check,
        "namespace",
        get("namespace"),
        &[
            ListItem::Str("html"),
            ListItem::Str("mathml"),
            ListItem::Str("svg"),
        ],
    )?;
    boolean(check, "modernAst", get("modernAst"))?;
    string(check, "outputFilename", get("outputFilename"))?;
    boolean(check, "preserveComments", get("preserveComments"))?;
    list(
        check,
        "fragments",
        get("fragments"),
        &[ListItem::Str("html"), ListItem::Str("tree")],
    )?;
    boolean(check, "preserveWhitespace", get("preserveWhitespace"))?;
    boolean(check, "hmr", get("hmr"))?;
    deprecated(
        check,
        get("enableSourcemap"),
        Code::options_removed_enable_sourcemap,
        messages::options_removed_enable_sourcemap(),
    )?;
    deprecated(
        check,
        get("hydratable"),
        Code::options_removed_hydratable,
        messages::options_removed_hydratable(),
    )?;
    removed(
        check,
        get("format"),
        "The format option has been removed in Svelte 4, the compiler only outputs ESM now. Remove \"format\" from your compiler options. If you did not set this yourself, bump the version of your bundler plugin (vite-plugin-svelte/rollup-plugin-svelte/svelte-loader)",
    )?;
    removed(
        check,
        get("tag"),
        "The tag option has been removed in Svelte 5. Use `<svelte:options customElement=\"tag-name\" />` inside the component instead. If that does not solve your use case, please open an issue on GitHub with details.",
    )?;
    removed(
        check,
        get("sveltePath"),
        "The sveltePath option has been removed in Svelte 5. If this option was crucial for you, please open an issue on GitHub with your use case.",
    )?;
    removed(
        check,
        get("errorMode"),
        "The errorMode option has been removed. If you are using this through svelte-preprocess with TypeScript, use the https://www.typescriptlang.org/tsconfig#verbatimModuleSyntax setting instead",
    )?;
    removed(
        check,
        get("varsReport"),
        "The vars option has been removed. If you are using this through svelte-preprocess with TypeScript, use the https://www.typescriptlang.org/tsconfig#verbatimModuleSyntax setting instead",
    )?;
    // Read during the analysis: `customElement` first, then `css`.
    let custom_element_error = match &custom_element {
        OptionValue::Undefined | OptionValue::Bool(_) | OptionValue::Function => None,
        OptionValue::Unknown => return Err(Undecided),
        _ => Some("customElement should be true or false".to_string()),
    };
    let css_error = match &css {
        OptionValue::Undefined | OptionValue::Function => None,
        OptionValue::Str(s) if s == "external" || s == "injected" => None,
        OptionValue::Unknown => return Err(Undecided),
        OptionValue::Bool(_) => Some(
            "The boolean options have been removed from the css option. Use \"external\" instead of false and \"injected\" instead of true".to_string(),
        ),
        OptionValue::Str(s) if s == "none" => Some(
            "css: \"none\" is no longer a valid option. If this was crucial for you, please open an issue on GitHub with your use case.".to_string(),
        ),
        _ => Some("css should be either \"external\" (default, recommended) or \"injected\"".to_string()),
    };
    check.late_error = custom_element_error
        .map(|m| (LateOption::CustomElement, m))
        .or(css_error.map(|m| (LateOption::Css, m)))
        .map(|(option, m)| {
            (
                option,
                Code::options_invalid_value,
                messages::options_invalid_value(&m),
            )
        });
    Ok(())
}

/// Every option `validate_component_options` knows.
fn is_component_option(key: &str) -> bool {
    matches!(
        key,
        "filename"
            | "rootDir"
            | "dev"
            | "generate"
            | "warningFilter"
            | "experimental"
            | "accessors"
            | "css"
            | "cssHash"
            | "cssOutputFilename"
            | "customElement"
            | "discloseVersion"
            | "immutable"
            | "legacy"
            | "compatibility"
            | "loopGuardTimeout"
            | "name"
            | "namespace"
            | "modernAst"
            | "outputFilename"
            | "preserveComments"
            | "fragments"
            | "preserveWhitespace"
            | "runes"
            | "hmr"
            | "sourcemap"
            | "enableSourcemap"
            | "hydratable"
            | "format"
            | "tag"
            | "sveltePath"
            | "errorMode"
            | "varsReport"
    )
}

fn fail(check: &mut CompileOptionsCheck, code: Code, message: String) -> Step {
    check.error = Some((code, message));
    Err(Undecided)
}

fn invalid(check: &mut CompileOptionsCheck, details: String) -> Step {
    fail(
        check,
        Code::options_invalid_value,
        messages::options_invalid_value(&details),
    )
}

fn boolean(check: &mut CompileOptionsCheck, keypath: &str, value: &OptionValue) -> Step {
    match value {
        OptionValue::Undefined | OptionValue::Bool(_) => Ok(()),
        OptionValue::Unknown => Err(Undecided),
        _ => invalid(
            check,
            format!("{keypath} should be true or false, if specified"),
        ),
    }
}

fn string(check: &mut CompileOptionsCheck, keypath: &str, value: &OptionValue) -> Step {
    match value {
        OptionValue::Undefined | OptionValue::Str(_) => Ok(()),
        OptionValue::Unknown => Err(Undecided),
        _ => invalid(check, format!("{keypath} should be a string, if specified")),
    }
}

fn fun(check: &mut CompileOptionsCheck, keypath: &str, value: &OptionValue) -> Step {
    match value {
        OptionValue::Undefined | OptionValue::Function => Ok(()),
        OptionValue::Unknown => Err(Undecided),
        _ => invalid(
            check,
            format!("{keypath} should be a function, if specified"),
        ),
    }
}

/// A nested option group with a single known `child` (the compiler's
/// `object(children)`): the group must be an object (or falsy) with no
/// other keys. Returns the value the child's validator receives —
/// `input && input[child]`.
fn group(
    check: &mut CompileOptionsCheck,
    keypath: &str,
    value: &OptionValue,
    child: &str,
) -> Result<OptionValue, Undecided> {
    match value {
        OptionValue::Undefined => Ok(OptionValue::Undefined),
        OptionValue::Unknown => Err(Undecided),
        OptionValue::Object(entries) => {
            for (key, _) in entries {
                if key != child {
                    fail(
                        check,
                        Code::options_unrecognised,
                        messages::options_unrecognised(&format!("{keypath}.{key}")),
                    )?;
                }
            }
            Ok(entries
                .iter()
                .rev()
                .find(|(k, _)| k == child)
                .map_or(OptionValue::Undefined, |(_, v)| v.clone()))
        }
        // Falsy values pass the shape check and reach the child as is.
        OptionValue::Null | OptionValue::Bool(false) => Ok(value.clone()),
        OptionValue::Str(s) if s.is_empty() => Ok(value.clone()),
        OptionValue::Num(n) if *n == 0.0 || n.is_nan() => Ok(value.clone()),
        _ => {
            invalid(check, format!("{keypath} should be an object"))?;
            Ok(OptionValue::Undefined)
        }
    }
}

enum ListItem {
    Str(&'static str),
    Num(f64),
}

fn list(
    check: &mut CompileOptionsCheck,
    keypath: &str,
    value: &OptionValue,
    options: &[ListItem],
) -> Step {
    let included = match value {
        OptionValue::Undefined => true,
        OptionValue::Unknown => return Err(Undecided),
        OptionValue::Str(s) => options
            .iter()
            .any(|o| matches!(o, ListItem::Str(x) if x == s)),
        OptionValue::Num(n) => options
            .iter()
            .any(|o| matches!(o, ListItem::Num(x) if x == n)),
        _ => false,
    };
    if included {
        return Ok(());
    }
    let show = |o: &ListItem| match o {
        ListItem::Str(s) => (*s).to_string(),
        ListItem::Num(n) => format!("{n}"),
    };
    let message = if options.len() > 2 {
        let head: Vec<String> = options[..options.len() - 1]
            .iter()
            .map(|o| format!("\"{}\"", show(o)))
            .collect();
        format!(
            "{keypath} should be one of {} or \"{}\"",
            head.join(", "),
            show(&options[options.len() - 1])
        )
    } else {
        format!(
            "{keypath} should be either \"{}\" or \"{}\"",
            show(&options[0]),
            show(&options[1])
        )
    };
    invalid(check, message)
}

fn removed(check: &mut CompileOptionsCheck, value: &OptionValue, message: &str) -> Step {
    match value {
        OptionValue::Undefined => Ok(()),
        OptionValue::Unknown => Err(Undecided),
        _ => fail(
            check,
            Code::options_removed,
            messages::options_removed(message),
        ),
    }
}

/// `deprecate` / `warn_removed`: any value other than `undefined`
/// warns.
fn deprecated(
    check: &mut CompileOptionsCheck,
    value: &OptionValue,
    code: Code,
    message: String,
) -> Step {
    match value {
        OptionValue::Undefined => Ok(()),
        OptionValue::Unknown => Err(Undecided),
        _ => {
            check.warnings.push((code, message));
            Ok(())
        }
    }
}
