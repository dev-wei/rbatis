use serde_json::Value;

pub struct Update{

}

impl Update{
    pub  fn eval(&self, table: &str, arg: Value) -> Result<String,String>{
        unimplemented!()
    }
}