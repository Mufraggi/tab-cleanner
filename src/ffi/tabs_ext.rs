use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// Raw FFI bindings for `chrome.tabs.group` and `chrome.tabs.ungroup`.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["chrome", "tabs"], js_name = group)]
    fn chrome_tabs_group(options: &JsValue) -> js_sys::Promise;

    #[wasm_bindgen(js_namespace = ["chrome", "tabs"], js_name = ungroup)]
    fn chrome_tabs_ungroup(tab_ids: &JsValue) -> js_sys::Promise;
}

/// Group the given tab IDs into a new or existing Chrome tab group.
///
/// - `tab_ids`: the list of tab ids to group together.
/// - `group_id`: if `Some`, the tabs are added to the existing group;
///               if `None`, a new group is created.
///
/// Returns the group id assigned by Chrome.
pub async fn create_tab_group(tab_ids: &[i32], group_id: Option<i32>) -> Result<i32, JsValue> {
    let opts = js_sys::Object::new();
    js_sys::Reflect::set(
        &opts,
        &JsValue::from_str("tabIds"),
        &serde_wasm_bindgen::to_value(tab_ids)?,
    )?;
    if let Some(gid) = group_id {
        js_sys::Reflect::set(
            &opts,
            &JsValue::from_str("groupId"),
            &JsValue::from(gid),
        )?;
    }
    let promise = chrome_tabs_group(&JsValue::from(opts));
    let result = JsFuture::from(promise).await?;
    result
        .as_f64()
        .map(|v| v as i32)
        .ok_or_else(|| JsValue::from_str("chrome.tabs.group did not return a number"))
}

/// Remove the given tab IDs from their current Chrome tab groups.
pub async fn ungroup_tabs(tab_ids: &[i32]) -> Result<(), JsValue> {
    let ids = serde_wasm_bindgen::to_value(tab_ids)?;
    let promise = chrome_tabs_ungroup(&ids);
    JsFuture::from(promise).await?;
    Ok(())
}
