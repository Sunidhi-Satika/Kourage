//! Safety analyzer for AI-generated commands.

/// Check whether a shell command is potentially destructive or dangerous.
pub fn is_destructive_command(raw_cmd: &str) -> bool {
    let cmd = raw_cmd.trim().to_lowercase();

    // Dangerous removal patterns
    if cmd.starts_with("rm ") || cmd.contains(" rm ") {
        if cmd.contains("-rf")
            || cmd.contains("-fr")
            || cmd.contains("-r")
            || cmd.contains("-f *")
            || cmd.contains(" *")
            || cmd.contains("--recursive")
            || cmd.contains("-r ")
            || cmd.contains("-R ")
        {
            return true;
        }
    }

    // Disk / Partition manipulation
    let disk_ops = [
        "mkfs", "fdisk", "parted", "gdisk", "sfdisk", "shred",
        "dd if=", "dd of=", "> /dev/sd", "> /dev/nvme", "> /dev/hd",
    ];
    for op in disk_ops {
        if cmd.contains(op) {
            return true;
        }
    }

    // Destructive Git operations
    if cmd.contains("git ") {
        if cmd.contains("reset") && cmd.contains("--hard") {
            return true;
        }
        if cmd.contains("clean") && (cmd.contains("-f") || cmd.contains("-fd") || cmd.contains("-x")) {
            return true;
        }
        if cmd.contains("push") && (cmd.contains("--force") || cmd.contains(" -f ") || cmd.ends_with(" -f")) {
            return true;
        }
        if cmd.contains("branch") && (cmd.contains(" -d") || cmd.contains(" -D")) {
            return true;
        }
    }

    // Dangerous permissions on root or broad directories
    if (cmd.contains("chmod ") || cmd.contains("chown ")) && (cmd.contains("-r") || cmd.contains("-R")) {
        if cmd.contains(" /") || cmd.contains(" /etc") || cmd.contains(" /var") || cmd.contains(" 777") {
            return true;
        }
    }

    // Database destruction
    let db_ops = ["drop table", "drop database", "truncate table", "drop schema"];
    for op in db_ops {
        if cmd.contains(op) {
            return true;
        }
    }

    // Fork bombs / Kernel crash triggers
    if cmd.contains(":(){ :|:& };:") || cmd.contains("/proc/sysrq-trigger") {
        return true;
    }

    // System power state changes
    let power_ops = ["shutdown", "reboot", "poweroff", "init 0", "init 6", "halt"];
    for op in power_ops {
        if cmd == op || cmd.starts_with(&format!("{op} ")) || cmd.contains(&format!(" {op} ")) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_destructive_rm_commands() {
        assert!(is_destructive_command("rm -rf /tmp/test"));
        assert!(is_destructive_command("sudo rm -fr /var/log/*"));
        assert!(is_destructive_command("rm -r node_modules"));
        assert!(is_destructive_command("rm *"));
        assert!(!is_destructive_command("rm file.txt"));
    }

    #[test]
    fn test_destructive_git_commands() {
        assert!(is_destructive_command("git reset --hard HEAD~1"));
        assert!(is_destructive_command("git clean -fdx"));
        assert!(is_destructive_command("git push origin master --force"));
        assert!(is_destructive_command("git push -f origin main"));
        assert!(!is_destructive_command("git status"));
        assert!(!is_destructive_command("git checkout -b feature"));
    }

    #[test]
    fn test_destructive_disk_and_sys_commands() {
        assert!(is_destructive_command("mkfs.ext4 /dev/sdb1"));
        assert!(is_destructive_command("dd if=/dev/zero of=/dev/sda"));
        assert!(is_destructive_command("shutdown -h now"));
        assert!(is_destructive_command("reboot"));
        assert!(!is_destructive_command("ls -la"));
        assert!(!is_destructive_command("cargo build --release"));
    }
}
