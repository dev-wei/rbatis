use std::collections::HashMap;
use std::hash::Hash;
use std::ops::{Deref, Index};
use std::sync::{Mutex, RwLock};

use serde_json::json;
use serde_json::Value;

use crate::core::Error;
use rexpr::ast::Node;
use rexpr::lexer::lexer;
use rexpr::runtime::RExprRuntime;
use crate::interpreter::sql::ast::RbatisAST;
use crate::interpreter::sql::node::bind_node::BindNode;
use crate::interpreter::sql::node::choose_node::ChooseNode;
use crate::interpreter::sql::node::foreach_node::ForEachNode;
use crate::interpreter::sql::node::if_node::IfNode;
use crate::interpreter::sql::node::node::do_child_nodes;
use crate::interpreter::sql::node::node_type::NodeType;
use crate::interpreter::sql::node::otherwise_node::OtherwiseNode;
use crate::interpreter::sql::node::print_node::PrintNode;
use crate::interpreter::sql::node::proxy_node::{NodeFactory, ProxyNode};
use crate::interpreter::sql::node::set_node::SetNode;
use crate::interpreter::sql::node::string_node::StringNode;
use crate::interpreter::sql::node::trim_node::TrimNode;
use crate::interpreter::sql::node::when_node::WhenNode;
use crate::interpreter::sql::node::where_node::WhereNode;
use dashmap::DashMap;

/// Py lang,make sure Send+Sync
#[derive(Debug)]
pub struct PyRuntime {
    pub cache: DashMap<String, Vec<NodeType>>,
    pub generate: Vec<Box<dyn NodeFactory>>,
}

impl PyRuntime {

    pub fn new(generate: Vec<Box<dyn NodeFactory>>) ->Self{
        Self{ cache: Default::default(), generate: generate }
    }
    ///eval with cache
    pub fn eval(&self, driver_type: &crate::core::db::DriverType, py_sql: &str, env: &mut Value, engine: &RExprRuntime) -> Result<(String, Vec<serde_json::Value>), Error> {
        if !env.is_object() {
            return Result::Err(Error::from("[rbatis] py_sql Requires that the parameter be an json object!"));
        }
        let nodes = self.parse_and_cache(py_sql)?;
        let mut sql = String::new();
        let mut arg_array = vec![];
        do_child_nodes(driver_type, &nodes, env, engine, &mut arg_array, &mut sql)?;
        sql = sql.trim().to_string();
        return Ok((sql, arg_array));
    }
    /// parser and cache py data sql,return an vec node type
    pub fn parse_and_cache(&self, arg: &str) -> Result<Vec<NodeType>, crate::core::Error> {
        let rd = self.cache.get(arg);
        if rd.is_none() {
            let nods = PyRuntime::parse(arg, &self.generate)?;
            self.try_cache_into(arg, nods.clone());
            return Ok(nods);
        } else {
            let nodes = rd.unwrap();
            return Ok(nodes.clone());
        }
    }

    fn try_cache_into(&self, py: &str, arg: Vec<NodeType>) -> Option<Vec<NodeType>> {
        let rd = self.cache.insert(py.to_string(), arg);
        return rd;
    }

    pub fn add_gen<T>(&mut self, arg: T) where T: NodeFactory + 'static {
        self.generate.push(Box::new(arg));
    }

    /// parser py string data
    pub fn parse(arg: &str, generates: &Vec<Box<dyn NodeFactory>>) -> Result<Vec<NodeType>, crate::core::Error> {
        let line_space_map = PyRuntime::create_line_space_map(arg);
        let mut main_node = vec![];
        let ls = arg.lines();
        let mut space = -1;
        let mut line = -1;
        let mut skip = -1;
        for x in ls {
            line += 1;
            if x.is_empty() || (skip != -1 && line <= skip) {
                continue;
            }
            let count_index = *line_space_map.get(&line).unwrap();
            if space == -1 {
                space = count_index;
            }
            let (child_str, do_skip) = PyRuntime::find_child_str(line, count_index, arg, &line_space_map);
            if do_skip != -1 && do_skip >= skip {
                skip = do_skip;
            }
            let parserd;
            if !child_str.is_empty() {
                parserd = PyRuntime::parse(child_str.as_str(), generates)?;
            } else {
                parserd = vec![];
            }
            PyRuntime::parse_node(generates, &mut main_node, x, *line_space_map.get(&line).unwrap() as usize, parserd)?;
        }
        return Ok(main_node);
    }

    fn parse_trim_node(factorys: &Vec<Box<dyn NodeFactory>>, trim_express: &str, main_node: &mut Vec<NodeType>, source_str: &str, space: usize, childs: Vec<NodeType>) -> Result<NodeType, crate::core::Error> {
        if trim_express.starts_with(IfNode::name()) {
            return Ok(NodeType::NIf(IfNode::from(trim_express, childs)?));
        } else if trim_express.starts_with(ForEachNode::name()) {
            return Ok(NodeType::NForEach(ForEachNode::from(source_str, &trim_express, childs)?));
        } else if trim_express.starts_with(TrimNode::name()) {
            return Ok(NodeType::NTrim(TrimNode::from(source_str, trim_express, childs)?));
        } else if trim_express.starts_with(ChooseNode::name()) {
            return Ok(NodeType::NChoose(ChooseNode::from(source_str, trim_express, childs)?));
        } else if trim_express.starts_with(OtherwiseNode::def_name()) || trim_express.starts_with(OtherwiseNode::name()) {
            return Ok(NodeType::NOtherwise(OtherwiseNode::from(source_str, trim_express, childs)?));
        } else if trim_express.starts_with(WhenNode::name()) {
            return Ok(NodeType::NWhen(WhenNode::from(source_str, trim_express, childs)?));
        } else if trim_express.starts_with(BindNode::def_name()) || trim_express.starts_with(BindNode::name()) {
            return Ok(NodeType::NBind(BindNode::from(source_str, trim_express, childs)?));
        } else if trim_express.starts_with(SetNode::name()) {
            return Ok(NodeType::NSet(SetNode::from(source_str, trim_express, childs)?));
        } else if trim_express.starts_with(WhereNode::name()) {
            return Ok(NodeType::NWhere(WhereNode::from(source_str, trim_express, childs)?));
        } else if trim_express.starts_with(PrintNode::name()) {
            return Ok(NodeType::NPrint(PrintNode::from(source_str, trim_express, childs)?));
        } else {
            for f in factorys {
                let gen = f.try_new(trim_express, childs.clone())?;
                if gen.is_some() {
                    return Ok(NodeType::NCustom(gen.unwrap()));
                }
            }
            // unkonw tag
            return Err(crate::core::Error::from("[rbatis] unknow tag: ".to_string() + source_str));
        }
    }


    fn parse_node(generates: &Vec<Box<dyn NodeFactory>>, main_node: &mut Vec<NodeType>, x: &str, space: usize, mut childs: Vec<NodeType>) -> Result<(), crate::core::Error> {
        let mut trim_x = x.trim();
        if trim_x.starts_with("//") {
            return Ok(());
        }
        if trim_x.ends_with(":") {
            trim_x = trim_x[0..trim_x.len() - 1].trim();
            if trim_x.contains(": ") {
                let vecs: Vec<&str> = trim_x.split(": ").collect();
                if vecs.len() > 1 {
                    let len = vecs.len();
                    for index in 0..len {
                        let index = len - 1 - index;
                        let item = vecs[index];
                        childs = vec![Self::parse_trim_node(generates, item, main_node, x, space, childs)?];
                        if index == 0 {
                            for x in &childs {
                                main_node.push(x.clone());
                            }
                            return Ok(());
                        }
                    }
                }
            }
            let node = Self::parse_trim_node(generates, trim_x, main_node, x, space, childs)?;
            main_node.push(node);
            return Ok(());
        } else {
            //string,replace space to only one
            let mut data = x.to_owned();
            if space <= 1 {
                data = x.to_string();
            } else {
                data = x[(space - 1)..].to_string();
            }
            main_node.push(NodeType::NString(StringNode::new(&data)));
            for x in childs {
                main_node.push(x);
            }
            return Ok(());
        }
    }

    fn count_space(arg: &str) -> i32 {
        let cs = arg.chars();
        let mut index = 0;
        for x in cs {
            match x {
                ' ' => {
                    index += 1;
                }
                _ => {
                    break;
                }
            }
        }
        return index;
    }

    ///find_child_str
    fn find_child_str(line_index: i32, space_index: i32, arg: &str, m: &HashMap<i32, i32>) -> (String, i32) {
        let mut result = String::new();
        let mut skip_line = -1;
        let mut line = -1;
        let lines = arg.lines();
        for x in lines {
            line += 1;
            if line > line_index {
                let cached_space = *m.get(&line).unwrap();
                if cached_space > space_index {
                    result = result + x + "\n";
                    skip_line = line;
                } else {
                    break;
                }
            }
        }
        let ss = result.as_str();
        return (result, skip_line);
    }

    ///Map<line,space>
    fn create_line_space_map(arg: &str) -> HashMap<i32, i32> {
        let mut m = HashMap::new();
        let lines = arg.lines();
        let mut line = -1;
        for x in lines {
            line += 1;
            let space = PyRuntime::count_space(x);
            //dothing
            m.insert(line, space);
        }
        return m;
    }
}

#[cfg(test)]
mod test {
    use crate::core::db::DriverType;
    use rexpr::runtime::RExprRuntime;
    use crate::interpreter::sql::node::node::do_child_nodes;
    use crate::interpreter::sql::py_sql::PyRuntime;
    use serde_json::Value;
    use crate::utils::bencher::QPS;

    #[test]
    pub fn test_py_interpreter_parse() {
        let s = "
    SELECT * FROM biz_activity
    //判断名称是否null
    if name!=null:
      AND delete_flag = #{del}
      AND version = 1
      if  age!=1:
        AND version = 1
      AND version = 1
    AND a = 0
      yes
    for item in ids:
      #{item}
    for index,item in ids:
      #{item}
    trim 'AND':
      AND delete_flag = #{del2}
    set:
        a=1
    where:
        id  = '2';";
        //println!("{}", s);
        let pys = PyRuntime::parse(s, &vec![]);
        match pys {
            Ok(v) => {
                println!("{:?}", v);
            }
            Err(e) => {
                println!("{:?}", e.to_string());
            }
        }
    }

    #[test]
    pub fn test_exec() {
        let s = "SELECT * FROM biz_activity where
    let info = 'yes':
    bind infos = 'ok':
    print info:
    print infos:
    if  name!=null:
      name = #{name}
    AND delete_flag1 = #{del}
    if  age != 1:
       AND age = 1
       if  age != 1:
         AND age = 2
    AND
    trim 'AND ':
      AND delete_flag2 = #{del}
    AND ids in (
    trim ',':
      for item in ids:
        #{item},
    )
    choose:
        when age == -1:
          AND age = -1
        _:
          age = 0
    choose:
        when age==27:
          AND age = 27
        otherwise:
          AND age = 0
    WHERE id  = 'end';";
        let pys = PyRuntime::parse(s, &vec![]).unwrap();
        println!("{:#?}", pys);
        let mut arg_array = vec![];
        let mut engine = RExprRuntime::new();
        let mut env = json!({
        "name": "1",
        "age": 27,
        "del":1,
        "ids":[1,2,3]
    });
        let mut r = String::new();
        do_child_nodes(&DriverType::Mysql, &pys, &mut env, &mut engine, &mut arg_array, &mut r).unwrap();
        println!("result sql:{}", r);
        println!("arg array:{:?}", arg_array.clone());
    }


    #[test]
    fn bind_test() {
        let mut env = json!({
        "name": "1",
        "age": 27,
        "del":1,
        "ids":[1,2,3]
    });

        let s = "
                       bind name=1+0:
                       if 1==1:
                              select 2
                       select ${name}
                         select 3
                           select 4
                       (
                       trim ',':
                         for item in ids:
                             ${item},
                       )
                       ";

        let pys = PyRuntime::parse(s, &vec![]).unwrap();
        println!("{:#?}", pys);

        let mut arg_array = vec![];
        let mut engine = RExprRuntime::new();
        let mut r = String::new();
        do_child_nodes(&DriverType::Mysql, &pys, &mut env, &mut engine, &mut arg_array, &mut r).unwrap();
        println!("result: {}", &r);
        println!("arg: {:?}", arg_array.clone());
    }

    #[test]
    fn test_find() {
        let s = "
                       bind name=1+0:
                       if 1==1:
                              select 2
                       select ${name}
                         select 3
                           select 4
                       ";
        let line_space_map = PyRuntime::create_line_space_map(s);
        println!("m:{:#?}", &line_space_map);
        let (child_str, do_skip) = PyRuntime::find_child_str(4,
                                                             "                       ".len() as i32,
                                                             s,
                                                             &line_space_map);

        println!("child_str: \n{}", &child_str);
        println!("skip: {}", do_skip);
    }

    #[test]
    pub fn test_print() {
        let s = "SELECT * FROM biz_activity where
                         print sql:
                             hello this is sql data
                         print arg_array:
                         print name:
                         print 1+1:
                       ";
        let pys = PyRuntime::parse(s, &vec![]).unwrap();
        let mut arg_array = vec![];
        let mut engine = RExprRuntime::new();
        let mut env = json!({ "name": "1", "age": 27 });
        let mut r = String::new();
        do_child_nodes(&DriverType::Mysql, &pys, &mut env, &mut engine, &mut arg_array, &mut r).unwrap();
        println!("result sql:{}", r.clone());
        println!("arg array:{:?}", arg_array.clone());
    }

    //cargo test --release --package rbatis --lib interpreter::sql::py_sql::test::test_bench --no-fail-fast -- --exact -Z unstable-options --show-output
    #[test]
    fn test_bench() {
        let runtime = PyRuntime::new(vec![]);
        let mut engine =RExprRuntime::new();

        //(Windows10 6Core16GBMem) use Time: 916.1591ms ,each:916 ns/op use QPS: 1091470 QPS/s
        let py_sql="SELECT * FROM biz_activity where
    if  name!=null:
      name = #{name}
    WHERE id  = 'end';";
        let mut env=serde_json::json!({});

        runtime.eval(&DriverType::Mysql, &py_sql, &mut env, &mut engine).unwrap();

        let total = 1000000;
        let now = std::time::Instant::now();
        for _ in 0..total {
            runtime.eval(&DriverType::Mysql, &py_sql, &mut env, &mut engine).unwrap();
        }
        now.time(total);
        now.qps(total);
    }
}