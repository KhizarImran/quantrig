use rustix::fs::{MemfdFlags, memfd_create};
use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
    SeccompRule, TargetArch,
};
use std::{
    error::Error,
    fs::File,
    io::{Seek, Write},
};

// The runner can use library threads that share its CPU and memory limits.
// Separate processes would each receive their own limits, so deny them.
pub(super) fn create() -> Result<[File; 2], Box<dyn Error>> {
    let arch = TargetArch::try_from(std::env::consts::ARCH)?;
    let thread = u64::try_from(libc::CLONE_THREAD)?;
    let separate_process = SeccompRule::new(vec![SeccompCondition::new(
        0,
        SeccompCmpArgLen::Dword,
        SeccompCmpOp::MaskedEq(thread),
        0,
    )?])?;
    let mut denied = [
        (libc::SYS_clone, vec![separate_process]),
        (libc::SYS_setsid, vec![]),
        (libc::SYS_setpgid, vec![]),
    ]
    .into_iter()
    .collect::<std::collections::BTreeMap<_, _>>();
    #[cfg(target_arch = "x86_64")]
    {
        // x32 uses the x86_64 audit architecture with a bit set in the syscall number.
        const X32: i64 = 0x4000_0000;
        denied.insert(libc::SYS_fork, vec![]);
        denied.insert(libc::SYS_vfork, vec![]);
        denied.extend(
            denied
                .keys()
                .map(|syscall| (syscall | X32, vec![]))
                .chain([(libc::SYS_clone3 | X32, vec![])])
                .collect::<Vec<_>>(),
        );
    }
    let process_filter = SeccompFilter::new(
        denied,
        SeccompAction::Allow,
        SeccompAction::KillProcess,
        arch,
    )?;

    // clone3 takes a pointer to its flags. ENOSYS prompts libc to use clone,
    // whose flags the filter can inspect.
    let clone3_fallback = SeccompFilter::new(
        [(libc::SYS_clone3, vec![])].into(),
        SeccompAction::Allow,
        SeccompAction::Errno(u32::try_from(libc::ENOSYS)?),
        arch,
    )?;
    Ok([filter_file(process_filter)?, filter_file(clone3_fallback)?])
}

fn filter_file(filter: SeccompFilter) -> Result<File, Box<dyn Error>> {
    let program = BpfProgram::try_from(filter)?;
    let mut file = File::from(memfd_create("quantrig-seccomp", MemfdFlags::CLOEXEC)?);
    for instruction in program {
        file.write_all(&instruction.code.to_ne_bytes())?;
        file.write_all(&[instruction.jt, instruction.jf])?;
        file.write_all(&instruction.k.to_ne_bytes())?;
    }
    file.rewind()?;
    Ok(file)
}
