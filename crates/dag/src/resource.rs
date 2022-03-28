use std::cell::RefCell;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use core::simulation::Simulation;

use compute::multicore::*;

pub struct Resource {
    pub id: String,
    pub compute: Rc<RefCell<Compute>>,
    pub speed: u64,
    pub cores_available: u32,
    pub memory_available: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct YamlResource {
    name: String,
    speed: u64,
    cores: u32,
    memory: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Resources {
    resources: Vec<YamlResource>,
}

pub fn load_resources(file: &str, sim: &mut Simulation) -> Vec<Resource> {
    let resources: Resources =
        serde_yaml::from_str(&std::fs::read_to_string(file).expect(&format!("Can't read file {}", file)))
            .expect(&format!("Can't parse YAML from file {}", file));
    let mut result: Vec<Resource> = Vec::new();
    for resource in resources.resources.into_iter() {
        let compute = Rc::new(RefCell::new(Compute::new(
            resource.speed,
            resource.cores,
            resource.memory,
            sim.create_context(&resource.name),
        )));
        sim.add_handler(&resource.name, compute.clone());
        result.push(Resource {
            id: resource.name,
            compute,
            speed: resource.speed,
            cores_available: resource.cores,
            memory_available: resource.memory,
        });
    }
    result
}