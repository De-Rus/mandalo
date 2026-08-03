pub mod proto;

pub use proto::{
    compile, decode, describe, encode, methods, GrpcMethodInfo, MessageShape, ProtoField,
    ProtoFieldType, ProtoFile,
};

#[cfg(target_arch = "wasm32")]
mod bindings {
    use super::proto;
    use serde::Serialize as _;
    use wasm_bindgen::prelude::*;

    fn files(value: JsValue) -> Result<Vec<proto::ProtoFile>, JsError> {
        serde_wasm_bindgen::from_value(value)
            .map_err(|e| JsError::new(&format!("proto files must be [{{ path, contents }}]: {e}")))
    }

    #[wasm_bindgen(js_name = compile)]
    pub fn compile(files_in: JsValue) -> Result<JsValue, JsError> {
        let listed = proto::methods(&files(files_in)?).map_err(|e| JsError::new(&e))?;
        serde_wasm_bindgen::to_value(&listed).map_err(|e| JsError::new(&e.to_string()))
    }

    #[wasm_bindgen(js_name = describe)]
    pub fn describe(files_in: JsValue, type_name: &str) -> Result<JsValue, JsError> {
        let shape = proto::describe(&files(files_in)?, type_name).map_err(|e| JsError::new(&e))?;
        // json_compatible, not the default: an unexpanded branch has to reach the page
        // as `null`, the contract's value, and not as `undefined`.
        shape
            .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
            .map_err(|e| JsError::new(&e.to_string()))
    }

    #[wasm_bindgen(js_name = encode)]
    pub fn encode(
        files_in: JsValue,
        service: &str,
        method: &str,
        json: &str,
    ) -> Result<Box<[u8]>, JsError> {
        proto::encode(&files(files_in)?, service, method, json)
            .map(Vec::into_boxed_slice)
            .map_err(|e| JsError::new(&e))
    }

    #[wasm_bindgen(js_name = decode)]
    pub fn decode(
        files_in: JsValue,
        service: &str,
        method: &str,
        bytes: &[u8],
    ) -> Result<String, JsError> {
        proto::decode(&files(files_in)?, service, method, bytes).map_err(|e| JsError::new(&e))
    }
}
