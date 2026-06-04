//! Shared JS payload fragments and wrappers used across vendor strategies.
//!
//! Anything in this file is byte-identical across at least two strategies
//! (Unity ESM, Unity CommonJS, Tuanjie). Strategy-specific scaffolding
//! (DI signatures, getInstance stubs, broadcast APIs, ...) stays inline in
//! the per-strategy modules.

/// Five `userInfo` fields shared verbatim by the Unity ESM strategy and the
/// Tuanjie strategy (12-space indentation, double-quoted values).
///
/// The CommonJS Unity strategy adds an extra `identifier` field and uses
/// single quotes, so it does NOT reuse this constant.
pub(crate) const FAKE_USER_FIELDS: &str = "            foreignKey: \"unifuck-fk\",
            name: \"Unifuck\",
            email: \"bypass@local\",
            primaryOrg: \"local\",
            createdAt: \"1970-01-01T00:00:00.000Z\",";

/// Wrap a body in the CommonJS `__awaiter` async wrapper used by the Unity
/// CommonJS strategy and the Tuanjie strategy.
///
/// Output template (with `<body>` replaced verbatim):
/// ```text
/// {
///         return __awaiter(this, void 0, void 0, function* () {
/// <body>
///         });
///     }
/// ```
///
/// `body` should already be indented to 12 spaces (the awaiter inner level).
pub(crate) fn wrap_awaiter(body: &str) -> String {
    format!(
        "{{\n        return __awaiter(this, void 0, void 0, function* () {{\n{body}\n        }});\n    }}"
    )
}

/// Wrap a body in the bare-braces pattern used by the Unity ESM strategy
/// (native `async`/`await`, no `__awaiter`) and by some Tuanjie methods.
///
/// Output template (with `<body>` replaced verbatim):
/// ```text
/// {
/// <body>
///     }
/// ```
///
/// `body` should already be indented to 8 spaces (the method inner level).
pub(crate) fn wrap_naked_async(body: &str) -> String {
    format!("{{\n{body}\n    }}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_awaiter_matches_legacy_is_license_valid() {
        let expected = r#"{
        return __awaiter(this, void 0, void 0, function* () {
            return true;
        });
    }"#;
        let got = wrap_awaiter("            return true;");
        assert_eq!(got, expected);
    }

    #[test]
    fn wrap_naked_async_matches_legacy_is_license_valid() {
        let expected = r#"{
        return true;
    }"#;
        let got = wrap_naked_async("        return true;");
        assert_eq!(got, expected);
    }

    #[test]
    fn fake_user_fields_matches_legacy_block() {
        let expected = r#"            foreignKey: "unifuck-fk",
            name: "Unifuck",
            email: "bypass@local",
            primaryOrg: "local",
            createdAt: "1970-01-01T00:00:00.000Z","#;
        assert_eq!(FAKE_USER_FIELDS, expected);
    }
}
