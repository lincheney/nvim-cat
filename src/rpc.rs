extern crate rmp;
extern crate rmp_serde;
extern crate serde;

use std::process::ChildStdout;
use self::serde::{Serialize, Deserialize};
use nvim::NvimError;

pub type MsgId = u32;
pub type Deserializer = rmp_serde::Deserializer<ChildStdout>;
pub type Serializer<'a> = rmp_serde::Serializer<'a, rmp_serde::encode::StructArrayWriter>;

pub struct Writer<'a> {
    msg_id:         MsgId,
    serializer:     Serializer<'a>,
}

pub struct Reader {
    deserializer: Deserializer,
}

impl<'a> Writer<'a> {
    pub fn new(serializer: Serializer<'a>) -> Self {
        Writer{ msg_id: 100, serializer: serializer }
    }

    pub fn write<T>(&mut self, command: &str, args: T) -> Result<MsgId, NvimError>
            where T: Serialize {

        self.msg_id += 1;
        let value = ( 0, self.msg_id, command, args );
        value.serialize(&mut self.serializer)?;
        Ok(self.msg_id)
    }
}

impl Reader {
    pub fn new(deserializer: Deserializer) -> Self {
        Reader{deserializer: deserializer}
    }

    pub fn read(&mut self) -> Result<Option<(u32, rmp::Value)>, NvimError> {
        let value: rmp_serde::Value = Deserialize::deserialize(&mut self.deserializer)?;
        let value = value.as_array().expect("expected an array");
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
