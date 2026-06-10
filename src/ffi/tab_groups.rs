use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// Raw FFI binding for `chrome.tabGroups.update`.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["chrome", "tabGroups"], js_name = update)]
    fn chrome_tab_groups_update(group_id: &JsValue, properties: &JsValue) -> js_sys::Promise;
}

/// Properties that can be updated on a Chrome tab group.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProperties {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Information returned by `chrome.tabGroups.update`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabGroupInfo {
    pub id: i32,
    pub collapsed: Option<bool>,
    pub color: Option<String>,
    pub title: Option<String>,
}

/// Update the colour and/or title of an existing Chrome tab group.
pub async fn update_tab_group(
    group_id: i32,
    properties: &UpdateProperties,
) -> Result<TabGroupInfo, JsValue> {
    let props = serde_wasm_bindgen::to_value(properties)?;
    let promise = chrome_tab_groups_update(&JsValue::from(group_id), &props);
    let result = JsFuture::from(promise).await?;
    let info: TabGroupInfo = serde_wasm_bindgen::from_value(result)?;
    Ok(info)
}
