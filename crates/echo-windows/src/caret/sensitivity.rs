//! Content-free TSF input-scope sensitivity policy.

use super::tsf_abi as abi;
use std::ffi::c_void;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sensitivity {
    Allowed,
    Denied,
    Unknown,
}

const IS_DEFAULT: u32 = 0;
const IS_TEXT: u32 = 57;
const IS_CHAT: u32 = 58;
const IS_PRIVATE: u32 = 61;
const IS_NUMERIC_PASSWORD: u32 = 63;
const IS_NUMERIC_PIN: u32 = 64;
const IS_ALPHANUMERIC_PIN: u32 = 65;
const IS_ALPHANUMERIC_PIN_SET: u32 = 66;
const IS_CHAT_WITHOUT_EMOJI: u32 = 68;

pub fn classify_scopes(scopes: &[u32]) -> Sensitivity {
    let mut allowed = false;
    for &scope in scopes {
        if matches!(
            scope,
            abi::IS_PASSWORD
                | IS_PRIVATE
                | IS_NUMERIC_PASSWORD
                | IS_NUMERIC_PIN
                | IS_ALPHANUMERIC_PIN
                | IS_ALPHANUMERIC_PIN_SET
        ) {
            return Sensitivity::Denied;
        }
        if matches!(scope, IS_TEXT | IS_CHAT | IS_CHAT_WITHOUT_EMOJI) {
            allowed = true;
        }
    }
    if allowed && !scopes.is_empty() {
        Sensitivity::Allowed
    } else if scopes.len() == 1 && scopes[0] == IS_DEFAULT {
        Sensitivity::Unknown
    } else {
        Sensitivity::Unknown
    }
}

/// Read only the input-scope metadata for the supplied range.  The caller
/// remains responsible for checking context status before this function.
/// The function never asks TSF for text, phrases, regular expressions or XML.
pub unsafe fn from_property(
    property: *mut c_void,
    ec: abi::TfEditCookie,
    range: *mut c_void,
) -> Sensitivity {
    if property.is_null() {
        return Sensitivity::Unknown;
    }
    let table = abi::vtable::<abi::ReadOnlyPropertyVtbl>(property);
    if table.is_null() {
        return Sensitivity::Unknown;
    }
    let mut value = abi::Variant::default();
    let hr = ((*table).get_value)(property, ec, range, &mut value);
    if hr < 0 || value.vt != abi::VT_UNKNOWN || value.data[0] == 0 {
        let _ = abi::VariantClear(&mut value);
        return Sensitivity::Unknown;
    }
    let unknown = value.data[0] as *mut c_void;
    let mut scope = std::ptr::null_mut();
    let qi = abi::vtable::<abi::UnknownVtbl>(unknown);
    let mut result = Sensitivity::Unknown;
    if !qi.is_null()
        && ((*qi).query_interface)(unknown, &abi::IID_ITF_INPUT_SCOPE, &mut scope) >= 0
        && !scope.is_null()
    {
        let scope_table = abi::vtable::<abi::InputScopeVtbl>(scope);
        if !scope_table.is_null() {
            let mut count = 0_u32;
            let mut values: *mut u32 = std::ptr::null_mut();
            let scope_hr = ((*scope_table).get_input_scopes)(scope, &mut values, &mut count);
            if scope_hr >= 0 && count <= 64 && (count == 0 || !values.is_null()) {
                let slice = if count == 0 {
                    &[][..]
                } else {
                    std::slice::from_raw_parts(values, count as usize)
                };
                result = classify_scopes(slice);
            }
            if !values.is_null() {
                abi::CoTaskMemFree(values.cast());
            }
        }
        abi::release(scope);
    }
    let _ = abi::VariantClear(&mut value);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denies_password_and_private_scopes() {
        for scope in [
            abi::IS_PASSWORD,
            IS_PRIVATE,
            IS_NUMERIC_PASSWORD,
            IS_NUMERIC_PIN,
        ] {
            assert_eq!(classify_scopes(&[scope]), Sensitivity::Denied);
        }
    }

    #[test]
    fn explicit_general_text_is_allowed() {
        assert_eq!(classify_scopes(&[IS_TEXT]), Sensitivity::Allowed);
        assert_eq!(
            classify_scopes(&[IS_CHAT, IS_CHAT_WITHOUT_EMOJI]),
            Sensitivity::Allowed
        );
    }

    #[test]
    fn absent_or_default_scope_is_unknown() {
        assert_eq!(classify_scopes(&[]), Sensitivity::Unknown);
        assert_eq!(classify_scopes(&[IS_DEFAULT]), Sensitivity::Unknown);
    }
}
