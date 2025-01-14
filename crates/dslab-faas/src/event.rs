use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct ContainerEndEvent {
    pub id: usize,
    pub expected_count: usize,
}

#[derive(Clone, Serialize)]
pub struct ContainerStartEvent {
    pub id: usize,
}

#[derive(Clone, Serialize)]
pub struct IdleDeployEvent {
    pub id: usize,
}

#[derive(Clone, Serialize)]
pub struct InvocationEndEvent {
    pub id: usize,
}

#[derive(Clone, Serialize)]
pub struct InvocationStartEvent {
    pub id: usize,
    pub func_id: usize,
}

#[derive(Clone, Serialize)]
pub struct SimulationEndEvent {}
