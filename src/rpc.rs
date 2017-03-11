extern crate rmp;
extern crate rmp_serde;
extern crate serde;

use std::process::ChildStdout;
use std::sync::Mutex;

use self::serde::{Serialize, Deserialize};
use nvim::NvimError;

pub type MsgId = u32;
pub type Deserializer = rmp_serde::Deserializer<ChildStdout>;
pub type Serializer<'a> = rmp_serde::Serializer<'a, rmp_serde::encode::StructArrayWriter>;

struct SerializeWrapper<'a> {
    msg_id:         MsgId,
    serializer:     Serializer<'a>,
}

pub struct Transport<'a> {
    deserializer:   Mutex<Deserializer>,
    serializer:     Mutex<SerializeWrapper<'a>>,
}

impl<'a> Transport<'a> {
    pub fn new(serializer: Serializer<'a>, deserializer: Deserializer) -> Self {
        Transport {
            serializer: Mutex::new(SerializeWrapper{msg_id: 100, serializer: serializer}),
            deserializer: Mutex::new(deserializer),
        }
    }

    pub fn send<T>(&mut self, command: &str, args: T) -> Result<MsgId, NvimError>
            where T: Serialize {

        let mut serializer = self.serializer.lock().unwrap();
        serializer.msg_id += 1;
        let id = serializer.msg_id;
        let value = ( 0, id, command, args );
        value.serialize(&mut serializer.serializer)?;
        Ok(id)
    }

    pub fn recv(&mut self) -> Result<Option<(u32, rmp::Value)>, NvimError> {
        let value: rmp_serde::Value = Deserialize::deserialize(&mut *self.deserializer.lock().unwrap())?;
        let value = value.as_array().expect("expected an array");
        // println!("\n{:?}", value);
        match value[0].as_u64().expect("expected an int") {
            1 => {
                // response
                let err_msg = if ! value[2].is_nil() {
                    Some(value[2].as_array().expect("expected an array")[1].as_str().expect("expected a string"))
                } else {
                    None
                };

                if let Some(err_msg) = err_msg {
                    Err(NvimError::RpcError(err_msg.to_string()))
                } else {
                    let id = value[1].as_u64().expect("expected an int") as u32;
                    Ok(Some( (id, value[3].clone()) ))
                }

            },
            // 2 => None, // notification
            _ => Ok(None),
        }
    }
}
