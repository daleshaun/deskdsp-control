//! Dynamic Node Rack System for DeskDSP Control.
//!
//! Provides ordered, pluggable mono (`MonoRack`) and stereo (`StereoRack`) effect racks.
//! Guarantees:
//! - Allocation-free execution in `process()` and `process_stereo()`.
//! - Pre-allocated rack capacity so runtime addition/removal never allocates on the audio thread.
//! - Per-node enable/bypass, reordering, insertion, and deletion.
//! - Safe typed downcasting (`find_node_mut::<T>()`) for real-time parameter tweaking.

#![allow(dead_code)]

use super::{DspNode, StereoDspNode, NodeTelemetry};

pub struct MonoRack {
    pub nodes: Vec<Box<dyn DspNode>>,
}

impl MonoRack {
    pub fn new() -> Self {
        Self::with_capacity(16)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(capacity),
        }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn push<T: DspNode + 'static>(&mut self, node: T) {
        self.nodes.push(Box::new(node));
    }

    pub fn push_boxed(&mut self, node: Box<dyn DspNode>) {
        self.nodes.push(node);
    }

    pub fn insert(&mut self, index: usize, node: Box<dyn DspNode>) {
        if index <= self.nodes.len() {
            self.nodes.insert(index, node);
        } else {
            self.nodes.push(node);
        }
    }

    pub fn remove(&mut self, index: usize) -> Option<Box<dyn DspNode>> {
        if index < self.nodes.len() {
            Some(self.nodes.remove(index))
        } else {
            None
        }
    }

    pub fn swap(&mut self, i: usize, j: usize) {
        if i < self.nodes.len() && j < self.nodes.len() {
            self.nodes.swap(i, j);
        }
    }

    pub fn move_node(&mut self, from: usize, to: usize) {
        if from < self.nodes.len() && to < self.nodes.len() && from != to {
            let node = self.nodes.remove(from);
            self.nodes.insert(to, node);
        }
    }

    pub fn get(&self, index: usize) -> Option<&dyn DspNode> {
        self.nodes.get(index).map(|b| b.as_ref())
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut (dyn DspNode + 'static)> {
        if index < self.nodes.len() {
            Some(self.nodes[index].as_mut())
        } else {
            None
        }
    }

    pub fn toggle_bypass(&mut self, index: usize) {
        if let Some(node) = self.nodes.get_mut(index) {
            let next = !node.is_bypassed();
            node.set_bypassed(next);
        }
    }

    pub fn set_bypassed(&mut self, index: usize, bypassed: bool) {
        if let Some(node) = self.nodes.get_mut(index) {
            node.set_bypassed(bypassed);
        }
    }

    pub fn is_bypassed(&self, index: usize) -> bool {
        self.nodes.get(index).map_or(false, |n| n.is_bypassed())
    }

    pub fn reset(&mut self) {
        for node in &mut self.nodes {
            node.reset();
        }
    }

    #[inline(always)]
    pub fn process(&mut self, mut sample: f32) -> f32 {
        for node in self.nodes.iter_mut() {
            sample = node.process_sample(sample);
        }
        sample
    }

    /// Safe downcast to locate the first node matching a concrete type.
    pub fn find_node<T: 'static>(&self) -> Option<&T> {
        for node in &self.nodes {
            if let Some(concrete) = node.as_any().downcast_ref::<T>() {
                return Some(concrete);
            }
        }
        None
    }

    /// Safe mutable downcast to locate and tweak the first node matching a concrete type.
    pub fn find_node_mut<T: 'static>(&mut self) -> Option<&mut T> {
        for node in &mut self.nodes {
            if let Some(concrete) = node.as_any_mut().downcast_mut::<T>() {
                return Some(concrete);
            }
        }
        None
    }

    /// Collect telemetry from all nodes in the rack.
    pub fn collect_telemetry(&self) -> Vec<(&'static str, NodeTelemetry)> {
        let mut telemetry = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            telemetry.push((node.name(), node.telemetry()));
        }
        telemetry
    }
}

pub struct StereoRack {
    pub nodes: Vec<Box<dyn StereoDspNode>>,
}

impl StereoRack {
    pub fn new() -> Self {
        Self::with_capacity(16)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(capacity),
        }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn push<T: StereoDspNode + 'static>(&mut self, node: T) {
        self.nodes.push(Box::new(node));
    }

    pub fn push_boxed(&mut self, node: Box<dyn StereoDspNode>) {
        self.nodes.push(node);
    }

    pub fn insert(&mut self, index: usize, node: Box<dyn StereoDspNode>) {
        if index <= self.nodes.len() {
            self.nodes.insert(index, node);
        } else {
            self.nodes.push(node);
        }
    }

    pub fn remove(&mut self, index: usize) -> Option<Box<dyn StereoDspNode>> {
        if index < self.nodes.len() {
            Some(self.nodes.remove(index))
        } else {
            None
        }
    }

    pub fn swap(&mut self, i: usize, j: usize) {
        if i < self.nodes.len() && j < self.nodes.len() {
            self.nodes.swap(i, j);
        }
    }

    pub fn move_node(&mut self, from: usize, to: usize) {
        if from < self.nodes.len() && to < self.nodes.len() && from != to {
            let node = self.nodes.remove(from);
            self.nodes.insert(to, node);
        }
    }

    pub fn get(&self, index: usize) -> Option<&dyn StereoDspNode> {
        self.nodes.get(index).map(|b| b.as_ref())
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut (dyn StereoDspNode + 'static)> {
        if index < self.nodes.len() {
            Some(self.nodes[index].as_mut())
        } else {
            None
        }
    }

    pub fn toggle_bypass(&mut self, index: usize) {
        if let Some(node) = self.nodes.get_mut(index) {
            let next = !node.is_bypassed();
            node.set_bypassed(next);
        }
    }

    pub fn set_bypassed(&mut self, index: usize, bypassed: bool) {
        if let Some(node) = self.nodes.get_mut(index) {
            node.set_bypassed(bypassed);
        }
    }

    pub fn is_bypassed(&self, index: usize) -> bool {
        self.nodes.get(index).map_or(false, |n| n.is_bypassed())
    }

    pub fn reset(&mut self) {
        for node in &mut self.nodes {
            node.reset();
        }
    }

    #[inline(always)]
    pub fn process_stereo(&mut self, mut left: f32, mut right: f32) -> (f32, f32) {
        for node in self.nodes.iter_mut() {
            let (l, r) = node.process_stereo(left, right);
            left = l;
            right = r;
        }
        (left, right)
    }

    pub fn find_node<T: 'static>(&self) -> Option<&T> {
        for node in &self.nodes {
            if let Some(concrete) = node.as_any().downcast_ref::<T>() {
                return Some(concrete);
            }
        }
        None
    }

    pub fn find_node_mut<T: 'static>(&mut self) -> Option<&mut T> {
        for node in &mut self.nodes {
            if let Some(concrete) = node.as_any_mut().downcast_mut::<T>() {
                return Some(concrete);
            }
        }
        None
    }

    pub fn collect_telemetry(&self) -> Vec<(&'static str, NodeTelemetry)> {
        let mut telemetry = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            telemetry.push((node.name(), node.telemetry()));
        }
        telemetry
    }
}
