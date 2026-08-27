//! Shadertoy shaders, translated well enough to run.
//!
//! There is a very large body of free, hand-written, GPU-only wallpaper out
//! there already, and almost all of it is GLSL written against Shadertoy's
//! conventions. Every one of those files is the lightest kind of wallpaper
//! Muivly can show — no decoder, no picture buffers, no codec threads — and
//! the only thing standing between them and a desktop is a dialect.
//!
//! So this is a dialect translator, not a GLSL compiler. It rewrites the
//! names that differ (`vec3` to `float3`, `mix` to `lerp`) and wraps
//! Shadertoy's `mainImage(out vec4, in vec2)` in the one-argument form
//! `procedural.rs` expects. What it deliberately does not do is understand
//! the language: a file using anything structural that HLSL spells
//! differently will fail to compile, and the compiler's message — with the
//! line numbers of the user's own file, because the translation is line for
//! line — is a better explanation than anything this could invent.
//!
//! Channels (`iChannel0`, `texture(...)`) are the one case worth naming
//! before the compiler gets to it: Muivly has no textures to bind, so a
//! shader that samples one cannot work at all, and "undeclared identifier
//! iChannel0" is not what that user needs to read.

/// Whether this file is GLSL rather than HLSL.
pub fn is_glsl(path: &std::path::Path) -> bool {
    let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(extension.to_ascii_lowercase().as_str(), "glsl" | "frag")
}

/// Words that mean the same thing under a different name. Order matters:
/// longer names first, so `vec4` is not eaten by a rule for `vec`.
const RENAMES: &[(&str, &str)] = &[
    ("vec2", "float2"),
    ("vec3", "float3"),
    ("vec4", "float4"),
    ("ivec2", "int2"),
    ("ivec3", "int3"),
    ("ivec4", "int4"),
    ("bvec2", "bool2"),
    ("bvec3", "bool3"),
    ("bvec4", "bool4"),
    ("mat2", "float2x2"),
    ("mat3", "float3x3"),
    ("mat4", "float4x4"),
    ("mix", "lerp"),
    ("fract", "frac"),
    ("mod", "fmod"),
    ("dFdx", "ddx"),
    ("dFdy", "ddy"),
    ("inversesqrt", "rsqrt"),
    ("atan", "atan2"),
    // Shadertoy's resolution is a float3 and ours is a float2. Everything
    // real uses `.xy`, and the prelude carries the third component.
    ("iResolution", "iResolutionXYZ"),
];

/// The message for a shader this cannot run, or `None` if it looks runnable.
pub fn unsupported(source: &str) -> Option<String> {
    if source.contains("iChannel") {
        return Some(
            "this shader samples a texture channel (iChannel0), and Muivly has \
             nothing to bind to one"
                .to_string(),
        );
    }
    if !source.contains("mainImage") {
        return Some("the shader has no mainImage function".to_string());
    }
    None
}

/// Turn a Shadertoy shader into something `procedural.rs` can compile.
///
/// Line for line: every rewrite happens within its own line and the wrapper
/// goes on the end, so a compiler error still points at the line the user
/// wrote. That is worth more than a tidier output.
pub fn translate(source: &str) -> String {
    let mut out = String::with_capacity(source.len() + 256);

    for line in source.lines() {
        // A preprocessor line is not code and must not be word-swapped: a
        // `#define mod(x,y)` helper would be rewritten into a definition of
        // `fmod`, which then shadows the real one.
        if line.trim_start().starts_with('#') {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        out.push_str(&rewrite_line(line));
        out.push('\n');
    }

    // Shadertoy writes its colour into an out parameter and measures its
    // coordinates in pixels. Ours returns a colour and measures 0-1.
    out.push_str(
        "\nfloat4 mainImage(float2 uv)\n\
         {\n\
         \x20   float4 colour = float4(0.0, 0.0, 0.0, 1.0);\n\
         \x20   muivlyShadertoyMain(colour, uv * iResolution);\n\
         \x20   return colour;\n\
         }\n",
    );

    out
}

/// Word replacements inside one line of code, plus the `mainImage` rename.
///
/// Whole words only, matched by hand rather than with a regex: the crate this
/// would otherwise need is a dependency, and the rule is short enough that
/// "is this character part of an identifier" is the whole of it.
fn rewrite_line(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len() + 16);
    let mut index = 0;

    while index < bytes.len() {
        if !is_word_start(bytes[index]) {
            out.push(bytes[index] as char);
            index += 1;
            continue;
        }

        let start = index;
        while index < bytes.len() && is_word(bytes[index]) {
            index += 1;
        }
        let word = &line[start..index];

        // The entry point is renamed rather than translated: the wrapper
        // appended at the end of the file is what carries its old name.
        if word == "mainImage" {
            out.push_str("muivlyShadertoyMain");
            continue;
        }

        match RENAMES.iter().find(|(from, _)| *from == word) {
            Some((_, to)) => out.push_str(to),
            None => out.push_str(word),
        }
    }

    out
}

fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn glsl_and_frag_are_translated_and_hlsl_is_not() {
        assert!(is_glsl(&PathBuf::from("waves.glsl")));
        assert!(is_glsl(&PathBuf::from(r"C:\x\WAVES.FRAG")));
        assert!(!is_glsl(&PathBuf::from("waves.hlsl")));
    }

    #[test]
    fn types_and_functions_are_renamed() {
        let out = rewrite_line("    vec3 c = mix(a, b, fract(t));");
        assert_eq!(out, "    float3 c = lerp(a, b, frac(t));");
    }

    /// The rule is whole words. A variable called `modifier` must survive a
    /// rename of `mod`, and `vec3fake` is not a `vec3`.
    #[test]
    fn only_whole_words_are_renamed() {
        assert_eq!(rewrite_line("modifier + vec3fake"), "modifier + vec3fake");
    }

    #[test]
    fn a_preprocessor_line_is_left_exactly_as_written() {
        let line = "#define mod289(x) x - floor(x)";
        assert_eq!(rewrite_line(line), rewrite_line(line));
        assert!(translate(line).starts_with(line));
    }

    #[test]
    fn the_entry_point_is_wrapped_rather_than_renamed_away() {
        let out = translate("void mainImage(out vec4 f, in vec2 p) { f = vec4(p, 0.0, 1.0); }");
        assert!(out.contains("void muivlyShadertoyMain(out float4 f, in float2 p)"));
        assert!(out.contains("float4 mainImage(float2 uv)"));
        assert!(out.contains("muivlyShadertoyMain(colour, uv * iResolution)"));
    }

    /// The translation is line for line so the compiler's line numbers still
    /// belong to the user's file. Only the wrapper is added, and only at the
    /// end.
    #[test]
    fn the_body_keeps_its_line_numbers() {
        let source = "// one\n// two\nvoid mainImage(out vec4 f, in vec2 p) {}\n";
        let out = translate(source);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "// one");
        assert_eq!(lines[1], "// two");
        assert!(lines[2].starts_with("void muivlyShadertoyMain"));
    }

    #[test]
    fn a_shader_that_needs_a_texture_says_so_before_the_compiler_does() {
        let message =
            unsupported("void mainImage(out vec4 f, in vec2 p){ f = texture(iChannel0, p); }");
        assert!(message.is_some_and(|m| m.contains("iChannel0")));
    }

    #[test]
    fn an_ordinary_shader_is_not_refused() {
        assert!(unsupported("void mainImage(out vec4 f, in vec2 p){ f = vec4(1.0); }").is_none());
    }
}
