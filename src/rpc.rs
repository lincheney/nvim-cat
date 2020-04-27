extern crate rmp;
extern crate rmpv;
extern crate rmp_serde;
extern crate serde;

use std::io::BufReader;
use std::process::{ChildStdout, ChildStdin};
use self::serde::{Serialize, Deserialize};
use nvim::NvimError;

pub type MsgId = u32;
pub type Serializer = rmp_serde::Serializer<ChildStdin>;
pub type Deserializer = rmp_serde::Deserializer<rmp_serde::decode::ReadReader<BufReader<ChildStdout>>>;

pub struct Writer {
    msg_id:         MsgId,
    serializer:     Serializer,
}

pub struct Reader {
    deserializer: Deserializer,
}

impl Writer {
    pub fn new(serializer: Serializer) -> Self {
        Writer{ msg_id: 100, serializer }
    }

    pub fn write<T: Serialize>(&mut self, command: &str, args: T) -> Result<MsgId, NvimError> {
        self.msg_id += 1;
        let value = ( 0, self.msg_id, command, args );
        value.serialize(&mut self.serializer)?;
        Ok(self.msg_id)
    }
}

impl Reader {
    pub fn new(reader: ChildStdout) -> Self {
        Reader{deserializer: Deserializer::new(BufReader::new(reader))}
    }

    pub fn read(&mut self) -> Result<Option<(u32, rmpv::Value)>, NvimError> {
        // let value = rmpv::decode::read_value(&mut self.reader)?;
        let value: rmpv::Value = Deserialize::deserialize(&mut self.deserializer)?;

        if let rmpv::Value::Array(value) = value {
            match value[0].as_u64().expect("expected an int") {
                1 => {
                    if ! value[2].is_nil() {
                        let error = value[2].as_array().expect("expected an array")[1].as_str().expect("expected a string");
                        return Err(NvimError::RpcError(error.to_string()))
                    }

                    let id = value[1].as_u64().expect("expected an int") as u32;
                    let value = value.into_iter().nth(3).unwrap();
                    Ok(Some( (id, value) ))

                },
                // 2 => None, // notification
                _ => Ok(None),
            }
        } else {
            panic!("sadf");
        }
    }
}
