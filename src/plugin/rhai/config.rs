use rhai::def_package;
use rhai::packages::{ArithmeticPackage, BasicArrayPackage, BasicMapPackage, LogicPackage};
use rhai::plugin::*;

#[export_module]
mod config_module {}
