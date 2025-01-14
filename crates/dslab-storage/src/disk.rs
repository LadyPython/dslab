//! Basic implementation of storage model for disk.
//!
//! It supports modeling of concurrent execution of disk operations by means of generic fair throughput sharing model
//! from the `dslab-models` crate. The underlying model also supports modeling of throughput degradation, variability
//! and dependence on operation properties by means of user-defined throughput and factor functions. For detailed
//! information about these functions, please refer to documentation in `dslab-models` crate.
//!
//! Note that this model is quite generic and can be used to model other types of storage as well.

use serde::Serialize;
use sugars::boxed;

use dslab_core::cast;
use dslab_core::component::Id;
use dslab_core::event::Event;
use dslab_core::handler::EventHandler;
use dslab_core::{context::SimulationContext, log_debug, log_error};
use dslab_models::throughput_sharing::{
    make_constant_throughput_fn, ActivityFactorFn, ConstantFactorFn, FairThroughputSharingModel, ResourceThroughputFn,
    ThroughputSharingModel,
};

use crate::events::{DataReadCompleted, DataReadFailed, DataWriteCompleted, DataWriteFailed};
use crate::storage::{Storage, StorageInfo};

/// Describes a disk operation.
#[derive(Clone)]
pub struct DiskActivity {
    /// Request Id.
    pub request_id: u64,
    /// Requester.
    pub requester: Id,
    /// Size.
    pub size: u64,
}

#[derive(Clone, Serialize)]
struct DiskReadActivityCompleted {}

#[derive(Clone, Serialize)]
struct DiskWriteActivityCompleted {}

type DiskThroughputModel = FairThroughputSharingModel<DiskActivity>;

///////////////////////////////////////////////////////////////////////////////

/// Disk builder. This is a type for convenient disk setup.
///
/// After disk settings are filled, [`DiskBuilder::build()`] should be called with [`SimulationContext`] to build a disk.
pub struct DiskBuilder {
    capacity: Option<u64>,
    read_throughput_fn: Option<ResourceThroughputFn>,
    write_throughput_fn: Option<ResourceThroughputFn>,
    read_factor_fn: Box<dyn ActivityFactorFn<DiskActivity>>,
    write_factor_fn: Box<dyn ActivityFactorFn<DiskActivity>>,
}

impl Default for DiskBuilder {
    /// Creates default disk builder.
    ///
    /// May be incomplete. User should fill required disk settings using other functions.
    fn default() -> Self {
        Self {
            capacity: None,
            read_throughput_fn: None,
            write_throughput_fn: None,
            read_factor_fn: boxed!(ConstantFactorFn::new(1.)),
            write_factor_fn: boxed!(ConstantFactorFn::new(1.)),
        }
    }
}

impl DiskBuilder {
    /// Same as [`DiskBuilder::default()`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates disk builder and fills it with given capacity, read and write bandwidth values.
    ///
    /// The underlying disk model uses constant throughput and factor functions.
    ///
    /// An alias for
    /// ```ignore
    /// DiskBuilder::new()
    ///     .capacity(capacity)
    ///     .constant_read_bw(read_bw)
    ///     .constant_write_bw(write_bw)
    /// ```
    pub fn simple(capacity: u64, read_bw: f64, write_bw: f64) -> Self {
        Self::new()
            .capacity(capacity)
            .constant_read_bw(read_bw)
            .constant_write_bw(write_bw)
    }

    /// Sets capacity of the disk.
    pub fn capacity(mut self, capacity: u64) -> Self {
        self.capacity.replace(capacity);
        self
    }

    /// Sets read bandwidth to be constant with given value.
    pub fn constant_read_bw(mut self, read_bw: f64) -> Self {
        self.read_throughput_fn.replace(make_constant_throughput_fn(read_bw));
        self
    }

    /// Sets write bandwidth to be constant with given value.
    pub fn constant_write_bw(mut self, write_bw: f64) -> Self {
        self.write_throughput_fn.replace(make_constant_throughput_fn(write_bw));
        self
    }

    /// Sets custom throughput function for read operations.
    pub fn read_throughput_fn(mut self, read_throughput_fn: ResourceThroughputFn) -> Self {
        self.read_throughput_fn.replace(read_throughput_fn);
        self
    }

    /// Sets custom throughput function for write operations.
    pub fn write_throughput_fn(mut self, write_throughput_fn: ResourceThroughputFn) -> Self {
        self.write_throughput_fn.replace(write_throughput_fn);
        self
    }

    /// Sets throughput factor function for read operations.
    pub fn read_factor_fn(mut self, read_factor_fn: Box<dyn ActivityFactorFn<DiskActivity>>) -> Self {
        self.read_factor_fn = read_factor_fn;
        self
    }

    /// Sets throughput factor function for write operations.
    pub fn write_factor_fn(mut self, write_factor_fn: Box<dyn ActivityFactorFn<DiskActivity>>) -> Self {
        self.write_factor_fn = write_factor_fn;
        self
    }

    /// Builds disk from given builder and simulation context.
    ///
    /// Panics on invalid or incomplete disk settings.
    pub fn build(self, ctx: SimulationContext) -> Disk {
        Disk {
            capacity: self.capacity.unwrap(),
            used: 0,
            read_throughput_model: FairThroughputSharingModel::new(
                self.read_throughput_fn.unwrap(),
                self.read_factor_fn,
            ),
            write_throughput_model: FairThroughputSharingModel::new(
                self.write_throughput_fn.unwrap(),
                self.write_factor_fn,
            ),
            next_request_id: 0,
            next_read_event: u64::MAX,
            next_write_event: u64::MAX,
            ctx,
        }
    }
}

///////////////////////////////////////////////////////////////////////////////

/// Represents a disk.
///
/// Disk is characterized by its capacity and read/write throughput models.
///
/// Disk state includes the amount of used disk space and state of throughput models.
/// Should be created using [`DiskBuilder`].
pub struct Disk {
    pub(in crate::disk) capacity: u64,
    pub(in crate::disk) used: u64,
    pub(in crate::disk) read_throughput_model: DiskThroughputModel,
    pub(in crate::disk) write_throughput_model: DiskThroughputModel,
    pub(in crate::disk) next_request_id: u64,
    pub(in crate::disk) next_read_event: u64,
    pub(in crate::disk) next_write_event: u64,
    pub(in crate::disk) ctx: SimulationContext,
}

impl Disk {
    fn make_unique_request_id(&mut self) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        request_id
    }

    fn schedule_next_read_event(&mut self) {
        if let Some((time, _)) = self.read_throughput_model.peek() {
            self.next_read_event = self.ctx.emit_self(DiskReadActivityCompleted {}, time - self.ctx.time());
        }
    }

    fn schedule_next_write_event(&mut self) {
        if let Some((time, _)) = self.write_throughput_model.peek() {
            self.next_write_event = self
                .ctx
                .emit_self(DiskWriteActivityCompleted {}, time - self.ctx.time());
        }
    }

    fn on_read_completed(&mut self) {
        let (_, activity) = self.read_throughput_model.pop().unwrap();
        self.ctx.emit_now(
            DataReadCompleted {
                request_id: activity.request_id,
                size: activity.size,
            },
            activity.requester,
        );
        self.schedule_next_read_event();
    }

    fn on_write_completed(&mut self) {
        let (_, activity) = self.write_throughput_model.pop().unwrap();
        self.ctx.emit_now(
            DataWriteCompleted {
                request_id: activity.request_id,
                size: activity.size,
            },
            activity.requester,
        );
        self.schedule_next_write_event();
    }
}

/// Storage model implementation for disk.
impl Storage for Disk {
    fn read(&mut self, size: u64, requester: Id) -> u64 {
        log_debug!(
            self.ctx,
            "Received read request, size: {}, requester: {}",
            size,
            requester
        );
        let request_id = self.make_unique_request_id();
        if size > self.capacity {
            let error = format!(
                "requested read size is {} but only {} is available",
                size, self.capacity
            );
            log_error!(self.ctx, "Failed reading: {}", error,);
            self.ctx.emit_now(DataReadFailed { request_id, error }, requester);
        } else {
            self.read_throughput_model.insert(
                DiskActivity {
                    request_id,
                    requester,
                    size,
                },
                size as f64,
                &mut self.ctx,
            );
            self.ctx.cancel_event(self.next_read_event);
            self.schedule_next_read_event();
        }
        request_id
    }

    fn write(&mut self, size: u64, requester: Id) -> u64 {
        let request_id = self.make_unique_request_id();
        log_debug!(
            self.ctx,
            "Received write request, size: {}, requester: {}",
            size,
            requester
        );
        let available = self.capacity - self.used;
        if available < size {
            let error = format!("requested write size is {} but only {} is available", size, available);
            log_error!(self.ctx, "Failed writing: {}", error,);
            self.ctx.emit_now(DataWriteFailed { request_id, error }, requester);
        } else {
            self.used += size;
            self.write_throughput_model.insert(
                DiskActivity {
                    request_id,
                    requester,
                    size,
                },
                size as f64,
                &mut self.ctx,
            );
            self.ctx.cancel_event(self.next_write_event);
            self.schedule_next_write_event();
        }
        request_id
    }

    fn mark_free(&mut self, size: u64) -> Result<(), String> {
        if size <= self.used {
            self.used -= size;
            return Ok(());
        }
        Err(format!("invalid size: {}", size))
    }

    fn used_space(&self) -> u64 {
        self.used
    }

    fn free_space(&self) -> u64 {
        self.capacity - self.used
    }

    fn capacity(&self) -> u64 {
        self.capacity
    }

    fn id(&self) -> Id {
        self.ctx.id()
    }

    fn info(&self) -> StorageInfo {
        StorageInfo {
            capacity: self.capacity(),
            used_space: self.used_space(),
            free_space: self.free_space(),
        }
    }
}

impl EventHandler for Disk {
    fn on(&mut self, event: Event) {
        cast!(match event.data {
            DiskReadActivityCompleted {} => {
                self.on_read_completed();
            }
            DiskWriteActivityCompleted {} => {
                self.on_write_completed();
            }
        })
    }
}
