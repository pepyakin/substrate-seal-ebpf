// This file is part of Substrate.

// Copyright (C) 2019-2022 Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

use solana_rbpf::{
	ebpf,
	elf::Executable,
	memory_region::{AccessType, MemoryMapping, MemoryRegion},
	verifier::RequisiteVerifier,
	vm::{Config, EbpfVm, SyscallRegistry, TestInstructionMeter, VerifiedExecutable},
};

pub struct MemoryRef<'a, 'b> {
	mapping: &'a mut MemoryMapping<'b>,
}

impl<'a, 'b> MemoryRef<'a, 'b> {
	pub fn erase(self) -> *mut () {
		self.mapping as *mut _ as *mut ()
	}

	pub unsafe fn recover(ptr: *mut ()) -> Self {
		MemoryRef { mapping: std::mem::transmute(ptr) }
	}

	pub fn read(&self, offset: u64, buf: &mut [u8]) {
		let host_addr = self.mapping.map(AccessType::Load, offset, buf.len() as u64).unwrap();
		buf.copy_from_slice(unsafe {
			std::slice::from_raw_parts(host_addr as usize as *mut u8, buf.len())
		});
	}

	pub fn write(&mut self, offset: u64, buf: &[u8]) {
		let host_addr = self.mapping.map(AccessType::Store, offset, buf.len() as u64).unwrap();
		unsafe {
			std::ptr::copy_nonoverlapping(buf.as_ptr(), host_addr as usize as *mut u8, buf.len())
		};
	}
}

/// This context is used for calling back into the supervisor.
pub trait SupervisorContext {
	fn supervisor_call(
		&mut self,
		r1: u64,
		r2: u64,
		r3: u64,
		r4: u64,
		r5: u64,
		memory_ref: MemoryRef<'_, '_>,
	) -> u64;
}

/// Executes the given program represented as an elf binary and input data.
pub fn execute(program: &[u8], input: &mut [u8], context: &mut dyn SupervisorContext) {
	let config = Config::default();
	let mut syscall_registry = SyscallRegistry::default();
	syscall_registry.register_syscall_by_name(b"abort", abort_syscall).unwrap();
	syscall_registry.register_syscall_by_name(b"ext_syscall", ext_syscall).unwrap();
	syscall_registry
		.register_syscall_by_name(b"sol_memcpy_", sol_memcpy_syscall)
		.unwrap();
	syscall_registry
		.register_syscall_by_name(b"sol_memmove_", sol_memmove_syscall)
		.unwrap();
	syscall_registry
		.register_syscall_by_name(b"sol_memset_", sol_memset_syscall)
		.unwrap();
	syscall_registry
		.register_syscall_by_name(b"sol_memcmp_", sol_memcmp_syscall)
		.unwrap();

	let executable =
		Executable::<TestInstructionMeter>::from_elf(program, config, syscall_registry).unwrap();
	let mem_region = MemoryRegion::new_writable(input, ebpf::MM_INPUT_START);
	let verified_executable =
		VerifiedExecutable::<RequisiteVerifier, TestInstructionMeter>::from_executable(executable)
			.unwrap();
	let mut vm =
		EbpfVm::new(&verified_executable, &mut ProcessData { context }, &mut [], vec![mem_region])
			.unwrap();
	let _res = vm
		.execute_program_interpreted(&mut TestInstructionMeter { remaining: 100_000_000 })
		.unwrap();
}

struct ProcessData<'a> {
	context: &'a mut dyn SupervisorContext,
}

fn abort_syscall(
	_invoke_context: &mut ProcessData,
	_arg1: u64,
	_arg2: u64,
	_arg3: u64,
	_arg4: u64,
	_arg5: u64,
	_memory_mapping: &mut MemoryMapping,
	result: &mut solana_rbpf::vm::ProgramResult,
) {
	let err = solana_rbpf::error::EbpfError::UserError(Box::new(AbortError));
	*result = solana_rbpf::vm::StableResult::Err(err);
}

#[derive(thiserror::Error, Debug)]
#[error("abort")]
struct AbortError;

fn ext_syscall(
	process_data: &mut ProcessData,
	arg1: u64,
	arg2: u64,
	arg3: u64,
	arg4: u64,
	arg5: u64,
	memory_mapping: &mut MemoryMapping,
	result: &mut solana_rbpf::vm::ProgramResult,
) {
	process_data.context.supervisor_call(
		arg1,
		arg2,
		arg3,
		arg4,
		arg5,
		MemoryRef { mapping: memory_mapping },
	);
	*result = solana_rbpf::vm::StableResult::Ok(0);
}

// pub fn sol_memcpy_(dest: *mut u8, src: *const u8, n: u64);
fn sol_memcpy_syscall(
	_process_data: &mut ProcessData,
	dest: u64,
	src: u64,
	n: u64,
	_arg4: u64,
	_arg5: u64,
	memory_mapping: &mut MemoryMapping,
	result: &mut solana_rbpf::vm::ProgramResult,
) {
	let mut buf = vec![0u8; n as usize];
	let mut memory_ref = MemoryRef { mapping: memory_mapping };
	memory_ref.read(src, &mut buf);
	memory_ref.write(dest, &buf);
	*result = solana_rbpf::vm::StableResult::Ok(0);
}

// pub fn sol_memmove_(dest: *mut u8, src: *const u8, n: u64);
fn sol_memmove_syscall(
	_process_data: &mut ProcessData,
	dest: u64,
	src: u64,
	n: u64,
	_arg4: u64,
	_arg5: u64,
	memory_mapping: &mut MemoryMapping,
	result: &mut solana_rbpf::vm::ProgramResult,
) {
	let mut buf = vec![0u8; n as usize];
	let mut memory_ref = MemoryRef { mapping: memory_mapping };
	memory_ref.read(src, &mut buf);
	memory_ref.write(dest, &buf);
	*result = solana_rbpf::vm::StableResult::Ok(0);
}

// pub fn sol_memset_(s: *mut u8, c: u8, n: u64);
fn sol_memset_syscall(
	_process_data: &mut ProcessData,
	s: u64,
	c: u64,
	n: u64,
	_arg4: u64,
	_arg5: u64,
	memory_mapping: &mut MemoryMapping,
	result: &mut solana_rbpf::vm::ProgramResult,
) {
	let buf = vec![c as u8; n as usize];
	let mut memory_ref = MemoryRef { mapping: memory_mapping };
	memory_ref.write(s, &buf);
	*result = solana_rbpf::vm::StableResult::Ok(0);
}

// pub fn sol_memcmp_(s1: *const u8, s2: *const u8, n: u64, result: *mut i32);
fn sol_memcmp_syscall(
	_process_data: &mut ProcessData,
	s1: u64,
	s2: u64,
	n: u64,
	result_ptr: u64,
	_arg5: u64,
	memory_mapping: &mut MemoryMapping,
	result: &mut solana_rbpf::vm::ProgramResult,
) {
	use std::cmp::Ordering;
	let mut buf1 = vec![0u8; n as usize];
	let mut buf2 = vec![0u8; n as usize];
	let mut memory_ref = MemoryRef { mapping: memory_mapping };
	memory_ref.read(s1, &mut buf1);
	memory_ref.read(s2, &mut buf2);
	match buf1.cmp(&buf2) {
		Ordering::Less => memory_ref.write(result_ptr, &(-1i32).to_le_bytes()),
		Ordering::Equal => memory_ref.write(result_ptr, &(0i32).to_le_bytes()),
		Ordering::Greater => memory_ref.write(result_ptr, &(1i32).to_le_bytes()),
	}
	*result = solana_rbpf::vm::StableResult::Ok(0);
}
