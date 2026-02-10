//! Linux kernel sandboxing: seccomp-BPF + Landlock.
//!
//! Applies OS-level restrictions that cannot be bypassed even if the
//! agent process is compromised. Complements the application-level
//! capability checker.
//!
//! - seccomp: restricts which syscalls the process can make
//! - landlock: restricts filesystem access to allowed paths

use std::ffi::CString;
use std::io;
use std::path::Path;

// ============================================================================
// Syscall numbers (x86_64)
// ============================================================================

const SYS_PRCTL: i64 = 157;
const SYS_SECCOMP: i64 = 317;
const SYS_LANDLOCK_CREATE_RULESET: i64 = 444;
const SYS_LANDLOCK_ADD_RULE: i64 = 445;
const SYS_LANDLOCK_RESTRICT_SELF: i64 = 446;

// ============================================================================
// prctl constants
// ============================================================================

const PR_SET_NO_NEW_PRIVS: i32 = 38;

// ============================================================================
// seccomp constants and BPF structures
// ============================================================================

const SECCOMP_SET_MODE_FILTER: u32 = 1;
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
const EPERM: u32 = 1;

// BPF instruction opcodes
const BPF_LD: u16 = 0x00;
const BPF_W: u16 = 0x00;
const BPF_ABS: u16 = 0x20;
const BPF_JMP: u16 = 0x05;
const BPF_JEQ: u16 = 0x10;
const BPF_K: u16 = 0x00;
const BPF_RET: u16 = 0x06;

/// BPF instruction.
#[repr(C)]
#[derive(Clone, Copy)]
struct SockFilter {
    code: u16,
    jt: u8,
    jf: u8,
    k: u32,
}

/// BPF program.
#[repr(C)]
struct SockFprog {
    len: u16,
    filter: *const SockFilter,
}

// seccomp_data offsets
const SECCOMP_DATA_NR: u32 = 0; // offset of syscall number in seccomp_data
const SECCOMP_DATA_ARCH: u32 = 4; // offset of architecture in seccomp_data
const AUDIT_ARCH_X86_64: u32 = 0xC000003E;

// ============================================================================
// Landlock constants and structures
// ============================================================================

const LANDLOCK_ACCESS_FS_EXECUTE: u64 = 1 << 0;
const LANDLOCK_ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
const LANDLOCK_ACCESS_FS_READ_FILE: u64 = 1 << 2;
const LANDLOCK_ACCESS_FS_READ_DIR: u64 = 1 << 3;
const LANDLOCK_ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
const LANDLOCK_ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
const LANDLOCK_ACCESS_FS_MAKE_CHAR: u64 = 1 << 6;
const LANDLOCK_ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
const LANDLOCK_ACCESS_FS_MAKE_REG: u64 = 1 << 8;
const LANDLOCK_ACCESS_FS_MAKE_SOCK: u64 = 1 << 9;
const LANDLOCK_ACCESS_FS_MAKE_FIFO: u64 = 1 << 10;
const LANDLOCK_ACCESS_FS_MAKE_BLOCK: u64 = 1 << 11;
const LANDLOCK_ACCESS_FS_MAKE_SYM: u64 = 1 << 12;

const LANDLOCK_RULE_PATH_BENEATH: u32 = 1;

/// All read access flags.
const LANDLOCK_READ_ALL: u64 = LANDLOCK_ACCESS_FS_READ_FILE | LANDLOCK_ACCESS_FS_READ_DIR;

/// All write access flags.
const LANDLOCK_WRITE_ALL: u64 = LANDLOCK_ACCESS_FS_WRITE_FILE
    | LANDLOCK_ACCESS_FS_REMOVE_DIR
    | LANDLOCK_ACCESS_FS_REMOVE_FILE
    | LANDLOCK_ACCESS_FS_MAKE_DIR
    | LANDLOCK_ACCESS_FS_MAKE_REG;

/// All access flags for landlock v1.
const LANDLOCK_ALL_V1: u64 = LANDLOCK_ACCESS_FS_EXECUTE
    | LANDLOCK_ACCESS_FS_WRITE_FILE
    | LANDLOCK_ACCESS_FS_READ_FILE
    | LANDLOCK_ACCESS_FS_READ_DIR
    | LANDLOCK_ACCESS_FS_REMOVE_DIR
    | LANDLOCK_ACCESS_FS_REMOVE_FILE
    | LANDLOCK_ACCESS_FS_MAKE_CHAR
    | LANDLOCK_ACCESS_FS_MAKE_DIR
    | LANDLOCK_ACCESS_FS_MAKE_REG
    | LANDLOCK_ACCESS_FS_MAKE_SOCK
    | LANDLOCK_ACCESS_FS_MAKE_FIFO
    | LANDLOCK_ACCESS_FS_MAKE_BLOCK
    | LANDLOCK_ACCESS_FS_MAKE_SYM;

#[repr(C)]
struct LandlockRulesetAttr {
    handled_access_fs: u64,
}

#[repr(C)]
struct LandlockPathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

// ============================================================================
// Raw syscall wrapper
// ============================================================================

unsafe fn syscall3(num: i64, a1: i64, a2: i64, a3: i64) -> i64 {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        inlateout("rax") num => ret,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack),
    );
    ret
}

unsafe fn syscall2(num: i64, a1: i64, a2: i64) -> i64 {
    syscall3(num, a1, a2, 0)
}

// ============================================================================
// Public API
// ============================================================================

/// Result of sandbox setup.
pub struct SandboxResult {
    pub seccomp_applied: bool,
    pub landlock_applied: bool,
    pub seccomp_error: Option<String>,
    pub landlock_error: Option<String>,
}

/// Apply seccomp + landlock sandboxing.
///
/// `read_paths`: paths the agent can read from
/// `write_paths`: paths the agent can write to
/// `enable_seccomp`: whether to apply seccomp BPF filter
/// `enable_landlock`: whether to apply landlock filesystem restrictions
pub fn apply_sandbox(
    read_paths: &[String],
    write_paths: &[String],
    enable_seccomp: bool,
    enable_landlock: bool,
) -> SandboxResult {
    let mut result = SandboxResult {
        seccomp_applied: false,
        landlock_applied: false,
        seccomp_error: None,
        landlock_error: None,
    };

    // Landlock must be applied BEFORE seccomp (seccomp may block landlock syscalls)
    if enable_landlock {
        match apply_landlock(read_paths, write_paths) {
            Ok(()) => {
                result.landlock_applied = true;
                eprintln!("sentinel: landlock sandbox active");
            }
            Err(e) => {
                result.landlock_error = Some(format!("{}", e));
                eprintln!("sentinel: landlock not available: {}", e);
            }
        }
    }

    if enable_seccomp {
        match apply_seccomp() {
            Ok(()) => {
                result.seccomp_applied = true;
                eprintln!("sentinel: seccomp sandbox active");
            }
            Err(e) => {
                result.seccomp_error = Some(format!("{}", e));
                eprintln!("sentinel: seccomp not available: {}", e);
            }
        }
    }

    result
}

// ============================================================================
// seccomp BPF implementation
// ============================================================================

/// Syscalls allowed by the seccomp filter.
/// This is the minimum set needed for Sentinel's operation.
const ALLOWED_SYSCALLS: &[u32] = &[
    // File I/O
    0,   // read
    1,   // write
    2,   // open
    3,   // close
    4,   // stat
    5,   // fstat
    6,   // lstat
    7,   // poll
    8,   // lseek
    9,   // mmap
    10,  // mprotect
    11,  // munmap
    12,  // brk
    13,  // rt_sigaction
    14,  // rt_sigprocmask
    16,  // ioctl (needed for terminal)
    17,  // pread64
    18,  // pwrite64
    19,  // readv
    20,  // writev
    21,  // access
    24,  // sched_yield
    28,  // madvise
    32,  // dup
    33,  // dup2
    35,  // nanosleep
    39,  // getpid
    41,  // socket
    42,  // connect
    44,  // sendto
    45,  // recvfrom
    46,  // sendmsg
    47,  // recvmsg
    49,  // bind (for localhost connections)
    54,  // setsockopt
    55,  // getsockopt
    56,  // clone (for thread creation)
    59,  // execve (for run_command tool)
    60,  // exit
    61,  // wait4
    62,  // kill (for process timeout)
    72,  // fcntl
    78,  // getdents
    79,  // getcwd
    80,  // chdir
    89,  // readlink
    96,  // gettimeofday
    97,  // getrlimit
    102, // getuid
    104, // getgid
    107, // geteuid
    108, // getegid
    110, // getppid
    131, // sigaltstack
    137, // statfs
    157, // prctl
    158, // arch_prctl
    202, // futex
    217, // getdents64
    218, // set_tid_address
    228, // clock_gettime
    230, // clock_nanosleep
    231, // exit_group
    233, // epoll_wait
    234, // tgkill
    257, // openat
    262, // newfstatat
    270, // pselect6
    271, // ppoll
    273, // set_robust_list
    281, // epoll_pwait
    288, // accept4
    291, // epoll_create1
    292, // dup3
    293, // pipe2
    302, // prlimit64
    309, // getcpu
    318, // getrandom
    332, // statx
    334, // rseq
    435, // clone3
    439, // faccessat2
];

fn apply_seccomp() -> Result<(), io::Error> {
    // Step 1: Set NO_NEW_PRIVS (required before seccomp filter)
    let ret = unsafe { syscall2(SYS_PRCTL, PR_SET_NO_NEW_PRIVS as i64, 1) };
    if ret != 0 {
        return Err(io::Error::from_raw_os_error(-ret as i32));
    }

    // Step 2: Build BPF filter
    let filter = build_seccomp_filter();

    // Step 3: Apply seccomp filter
    let prog = SockFprog {
        len: filter.len() as u16,
        filter: filter.as_ptr(),
    };

    let ret = unsafe {
        syscall3(
            SYS_SECCOMP,
            SECCOMP_SET_MODE_FILTER as i64,
            0,
            &prog as *const SockFprog as i64,
        )
    };

    if ret != 0 {
        return Err(io::Error::from_raw_os_error(-ret as i32));
    }

    Ok(())
}

fn build_seccomp_filter() -> Vec<SockFilter> {
    let mut filter = Vec::with_capacity(ALLOWED_SYSCALLS.len() + 5);

    // Verify architecture is x86_64
    filter.push(SockFilter {
        code: BPF_LD | BPF_W | BPF_ABS,
        jt: 0,
        jf: 0,
        k: SECCOMP_DATA_ARCH,
    });
    filter.push(SockFilter {
        code: BPF_JMP | BPF_JEQ | BPF_K,
        jt: 1,
        jf: 0,
        k: AUDIT_ARCH_X86_64,
    });
    // Kill on wrong architecture
    filter.push(SockFilter {
        code: BPF_RET | BPF_K,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_ERRNO | EPERM,
    });

    // Load syscall number
    filter.push(SockFilter {
        code: BPF_LD | BPF_W | BPF_ABS,
        jt: 0,
        jf: 0,
        k: SECCOMP_DATA_NR,
    });

    // For each allowed syscall, jump to ALLOW if match
    let num_checks = ALLOWED_SYSCALLS.len();
    for (i, &nr) in ALLOWED_SYSCALLS.iter().enumerate() {
        let remaining = num_checks - i - 1;
        filter.push(SockFilter {
            code: BPF_JMP | BPF_JEQ | BPF_K,
            jt: (remaining + 1) as u8, // jump to ALLOW (past remaining checks + DENY)
            jf: 0,                      // continue checking
            k: nr,
        });
    }

    // Default: DENY with EPERM
    filter.push(SockFilter {
        code: BPF_RET | BPF_K,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_ERRNO | EPERM,
    });

    // ALLOW
    filter.push(SockFilter {
        code: BPF_RET | BPF_K,
        jt: 0,
        jf: 0,
        k: SECCOMP_RET_ALLOW,
    });

    filter
}

// ============================================================================
// Landlock implementation
// ============================================================================

fn apply_landlock(read_paths: &[String], write_paths: &[String]) -> Result<(), io::Error> {
    // Step 1: Create ruleset
    let attr = LandlockRulesetAttr {
        handled_access_fs: LANDLOCK_ALL_V1,
    };

    let ruleset_fd = unsafe {
        syscall3(
            SYS_LANDLOCK_CREATE_RULESET,
            &attr as *const LandlockRulesetAttr as i64,
            std::mem::size_of::<LandlockRulesetAttr>() as i64,
            0,
        )
    };

    if ruleset_fd < 0 {
        return Err(io::Error::from_raw_os_error(-ruleset_fd as i32));
    }

    let ruleset_fd = ruleset_fd as i32;

    // Step 2: Add read-only rules
    for path in read_paths {
        if let Err(e) = add_landlock_path_rule(ruleset_fd, path, LANDLOCK_READ_ALL) {
            eprintln!("sentinel: landlock: failed to add read rule for {}: {}", path, e);
        }
    }

    // Step 3: Add read-write rules
    for path in write_paths {
        if let Err(e) = add_landlock_path_rule(
            ruleset_fd,
            path,
            LANDLOCK_READ_ALL | LANDLOCK_WRITE_ALL,
        ) {
            eprintln!("sentinel: landlock: failed to add write rule for {}: {}", path, e);
        }
    }

    // Always allow reading standard paths needed for operation
    let system_read_paths = [
        "/etc/resolv.conf",
        "/etc/hosts",
        "/etc/ssl",
        "/etc/ca-certificates",
        "/usr/share/ca-certificates",
        "/usr/lib",
        "/lib",
        "/lib64",
        "/proc/self",
    ];
    for path in &system_read_paths {
        let _ = add_landlock_path_rule(ruleset_fd, path, LANDLOCK_READ_ALL);
    }

    // Allow executing standard paths for run_command tool
    let exec_paths = ["/usr/bin", "/usr/local/bin", "/bin", "/usr/sbin"];
    for path in &exec_paths {
        let _ = add_landlock_path_rule(
            ruleset_fd,
            path,
            LANDLOCK_READ_ALL | LANDLOCK_ACCESS_FS_EXECUTE,
        );
    }

    // Allow /tmp for temporary files
    let _ = add_landlock_path_rule(
        ruleset_fd,
        "/tmp",
        LANDLOCK_READ_ALL | LANDLOCK_WRITE_ALL,
    );

    // Step 4: Set NO_NEW_PRIVS (required for landlock)
    let ret = unsafe { syscall2(SYS_PRCTL, PR_SET_NO_NEW_PRIVS as i64, 1) };
    if ret != 0 {
        unsafe { close_fd(ruleset_fd); }
        return Err(io::Error::from_raw_os_error(-ret as i32));
    }

    // Step 5: Enforce ruleset
    let ret = unsafe {
        syscall3(SYS_LANDLOCK_RESTRICT_SELF, ruleset_fd as i64, 0, 0)
    };

    unsafe { close_fd(ruleset_fd); }

    if ret != 0 {
        return Err(io::Error::from_raw_os_error(-ret as i32));
    }

    Ok(())
}

fn add_landlock_path_rule(
    ruleset_fd: i32,
    path: &str,
    access: u64,
) -> Result<(), io::Error> {
    // Verify path exists before trying to add rule
    if !Path::new(path).exists() {
        return Ok(()); // Skip non-existent paths silently
    }

    let c_path = CString::new(path.as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "path contains null byte")
    })?;

    // Open the path with O_PATH (just for the fd, no access check)
    let fd = unsafe {
        libc_open(c_path.as_ptr(), 0x200000 | 0x100000) // O_PATH | O_CLOEXEC
    };

    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    let path_attr = LandlockPathBeneathAttr {
        allowed_access: access,
        parent_fd: fd,
    };

    let ret = unsafe {
        syscall3(
            SYS_LANDLOCK_ADD_RULE,
            ruleset_fd as i64,
            LANDLOCK_RULE_PATH_BENEATH as i64,
            &path_attr as *const LandlockPathBeneathAttr as i64,
        )
    };

    unsafe { close_fd(fd); }

    if ret != 0 {
        return Err(io::Error::from_raw_os_error(-ret as i32));
    }

    Ok(())
}

// Open/close via libc (already linked by std).
extern "C" {
    fn open(pathname: *const i8, flags: i32, ...) -> i32;
    fn close(fd: i32) -> i32;
}

unsafe fn libc_open(path: *const i8, flags: i32) -> i32 {
    open(path, flags)
}

unsafe fn close_fd(fd: i32) {
    close(fd);
}
