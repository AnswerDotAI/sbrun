#!/usr/bin/perl

use strict;
use warnings;

use Cwd qw(realpath);
use Errno qw(EACCES ENOENT ENOTDIR EEXIST EPERM);
use POSIX qw(_SC_OPEN_MAX sysconf ttyname);

use constant F_GETPATH => 50;
use constant DEFAULT_XDG_CONFIG_DIRS => "/opt/homebrew/etc/xdg:/usr/local/etc/xdg:/etc/xdg";
use constant BUILTIN_VERSION => "0.0.3";
use constant DEFAULT_USER_CONFIG => <<'EOF';
# Default writable allow-list for sbrun.
#
# These entries are intentionally narrower than "your whole home directory":
# they try to cover common state, cache, and notebook/tool locations while
# still avoiding broad write access to arbitrary files under $HOME.
#
# Optional paths that do not resolve to directories are ignored.
# Add project- or tool-specific paths here or via -w/--writable.
#
# User config:
#   $XDG_CONFIG_HOME/sbrun/config
#   ~/.config/sbrun/config

optional_writable_dir=/tmp
optional_writable_dir=~/.cache
optional_writable_dir=~/.config
optional_writable_dir=~/.local/share
optional_writable_dir=~/.local/state
optional_writable_dir=~/.ipython
optional_writable_dir=~/.jupyter
optional_writable_dir=~/.matplotlib
optional_writable_dir=~/.npm
optional_writable_dir=~/.pnpm-store
optional_writable_dir=~/.cargo
optional_writable_dir=~/.rustup
optional_writable_dir=~/.conda
optional_writable_dir=~/Library/Caches
optional_writable_dir=~/Library/Logs
optional_writable_dir=~/Library/Jupyter
optional_writable_dir=~/Library/Python
optional_writable_dir=~/Library/Application Support/Jupyter
EOF

my $prog_name = do {
    my $name = $0;
    $name =~ s{.*/}{};
    $name;
};

sub dief {
    my ($fmt, @args) = @_;
    printf STDERR "%s: %s\n", $prog_name, sprintf($fmt, @args);
    exit 111;
}

sub die_errno {
    my ($what) = @_;
    dief("%s: %s", $what, $!);
}

sub path_join {
    my ($a, $b) = @_;
    return $a =~ m{/\z} ? $a . $b : $a . "/" . $b;
}

sub base_name {
    my ($path) = @_;
    $path =~ s{.*/}{};
    return $path;
}

sub print_help {
    my ($fh) = @_;
    print {$fh}
        "Usage: $prog_name [options] [command [args...]]\n",
        "\n",
        "Run commands under macOS sandbox-exec with writes confined to the\n",
        "current directory tree plus any configured extra writable paths.\n",
        "\n",
        "Options:\n",
        "  -h, --help           Show this help and exit\n",
        "  --version            Show version and exit\n",
        "  -w, --writable PATH  Allow writes to PATH; may be repeated\n",
        "  -e, --envdir VAR     Set VAR to .sbrun/VAR; may be repeated\n",
        "  -v, --unsetenv VAR   Remove VAR from the child environment; may be repeated\n",
        "  --                   Stop parsing $prog_name options and force command mode\n",
        "\n",
        "Behavior:\n",
        "  With no command, start \$SHELL as an interactive login shell.\n",
        "  If the first non-option argument starts with '-', pass arguments to \$SHELL.\n",
        "  Otherwise run the given command directly.\n",
        "\n",
        "Config:\n",
        "  Global config: \$XDG_CONFIG_DIRS/.../sbrun/config\n",
        "  User config:   \$XDG_CONFIG_HOME/sbrun/config or ~/.config/sbrun/config\n",
        "  Format: writable_path=/path or optional_writable_path=/path\n",
        "          writable_dir=/path and optional_writable_dir=/path are also accepted\n",
        "  Envdir:   -e/--envdir VAR is CLI-only; VAR must be [A-Za-z_][A-Za-z0-9_]*\n",
        "  Unsetenv: -v/--unsetenv VAR is CLI-only; some names are reserved\n";
}

sub program_version {
    my $script = realpath($0);
    if (defined($script) && $script =~ m{\A(.+)/[^/]+\z}) {
        my $path = path_join($1, "VERSION");
        if (open my $fh, "<", $path) {
            my $version = <$fh>;
            close $fh;
            if (defined($version)) {
                chomp $version;
                return $version if $version ne "";
            }
        }
    }
    return BUILTIN_VERSION;
}

sub print_version {
    my ($fh) = @_;
    print {$fh} "$prog_name ", program_version(), "\n";
}

sub cli_requests_help {
    my (@argv) = @_;
    my $i = 0;

    while ($i < @argv) {
        return 1 if $argv[$i] eq "-h" || $argv[$i] eq "--help";
        return 0 if $argv[$i] eq "--";
        if ($argv[$i] eq "-w" || $argv[$i] eq "--writable") {
            $i += 2;
            next;
        }
        if ($argv[$i] eq "-e" || $argv[$i] eq "--envdir") {
            $i += 2;
            next;
        }
        if ($argv[$i] eq "-v" || $argv[$i] eq "--unsetenv") {
            $i += 2;
            next;
        }
        if ($argv[$i] =~ /\A--writable=/) {
            ++$i;
            next;
        }
        if ($argv[$i] =~ /\A--envdir=/) {
            ++$i;
            next;
        }
        if ($argv[$i] =~ /\A--unsetenv=/) {
            ++$i;
            next;
        }
        last;
    }
    return 0;
}

sub cli_requests_version {
    my (@argv) = @_;
    my $i = 0;

    while ($i < @argv) {
        return 1 if $argv[$i] eq "--version";
        return 0 if $argv[$i] eq "--";
        if ($argv[$i] eq "-w" || $argv[$i] eq "--writable" || $argv[$i] eq "-e" || $argv[$i] eq "--envdir" ||
            $argv[$i] eq "-v" || $argv[$i] eq "--unsetenv") {
            $i += 2;
            next;
        }
        if ($argv[$i] =~ /\A--writable=/ || $argv[$i] =~ /\A--envdir=/ || $argv[$i] =~ /\A--unsetenv=/) {
            ++$i;
            next;
        }
        last;
    }
    return 0;
}

sub shell_is_bash {
    my ($shell_path) = @_;
    return base_name($shell_path) eq "bash";
}

sub history_file_name_for_shell {
    my ($shell_path) = @_;
    my $name = base_name($shell_path);
    return ".bash_history" if $name eq "bash";
    return ".zsh_history" if $name eq "zsh";
    return ".sh_history";
}

sub pick_shell {
    my ($env_shell, $pw_shell) = @_;
    return $env_shell if defined($env_shell) && $env_shell =~ m{\A/} && -x $env_shell;
    return $pw_shell if defined($pw_shell) && $pw_shell =~ m{\A/} && -x $pw_shell;
    return "/bin/bash" if -x "/bin/bash";
    dief("could not find an executable shell from \$SHELL, passwd entry, or /bin/bash");
}

sub config_path {
    my ($host_home, $xdg_config_home) = @_;
    return path_join(path_join($xdg_config_home, "sbrun"), "config")
        if defined($xdg_config_home) && $xdg_config_home =~ m{\A/};
    return path_join(path_join(path_join($host_home, ".config"), "sbrun"), "config")
        if defined($host_home) && $host_home ne "";
    return undef;
}

sub ensure_user_config_exists {
    my ($host_home, $xdg_config_home) = @_;
    my $path = config_path($host_home, $xdg_config_home);
    return if !defined($path) || -e $path;

    my $dir = $path;
    $dir =~ s{/[^/]+\z}{};
    if (!-d $dir) {
        require File::Path;
        File::Path::make_path($dir, { mode => 0700 });
    }

    if (open my $fh, ">", $path) {
        print {$fh} DEFAULT_USER_CONFIG or die_errno($path);
        close $fh or die_errno($path);
    }
}

sub default_global_config_dirs {
    my @dirs;
    my $script = realpath($0);
    push @dirs, path_join($1, "etc/xdg")
        if defined($script) && $script =~ m{\A(.+)/bin/[^/]+\z};
    push @dirs, split /:/, DEFAULT_XDG_CONFIG_DIRS;

    my %seen;
    return grep { $_ =~ m{\A/} && !$seen{$_}++ } @dirs;
}

sub global_config_paths {
    my @roots = defined($ENV{XDG_CONFIG_DIRS}) && $ENV{XDG_CONFIG_DIRS} ne ""
        ? grep { $_ =~ m{\A/} } split(/:/, $ENV{XDG_CONFIG_DIRS})
        : default_global_config_dirs();
    return map { path_join(path_join($_, "sbrun"), "config") } @roots;
}

sub trim_whitespace {
    my ($s) = @_;
    $s =~ s/\A\s+//;
    $s =~ s/\s+\z//;
    return $s;
}

sub expand_home_path {
    my ($path, $host_home) = @_;
    return $path unless $path =~ /\A~/;
    dief("cannot expand %s without a home directory", $path)
        if !defined($host_home) || $host_home eq "";
    return $host_home if $path eq "~";
    return path_join($host_home, substr($path, 2)) if $path =~ m{\A~/};
    dief("unsupported home expansion in path %s", $path);
}

sub resolve_writable_path {
    my ($path, $host_home, $optional) = @_;
    my $expanded = expand_home_path($path, $host_home);
    if (!-e $expanded) {
        return undef if $optional;
        local $! = ENOENT;
        dief("additional writable path %s: %s", $path, $!);
    }
    my $resolved = realpath($expanded);
    return undef if !defined($resolved) && $optional && ($! == ENOENT || $! == ENOTDIR);
    dief("additional writable path %s: %s", $path, $!)
        if !defined($resolved);
    return ($resolved, "dir") if -d $resolved;
    return ($resolved, "file") if -f $resolved;
    return undef if $optional;
    dief("additional writable path %s resolves to %s, which is not a regular file or directory", $path, $resolved);
}

sub add_writable_path {
    my ($dirs, $files, $seen, $path, $host_home, $optional) = @_;
    my ($resolved, $kind) = resolve_writable_path($path, $host_home, $optional);
    return if !defined($resolved);
    return if $seen->{$resolved};
    push @{$kind eq "dir" ? $dirs : $files}, $resolved;
    $seen->{$resolved} = 1;
}

sub load_config_writable_paths_from_path {
    my ($dirs, $files, $seen, $host_home, $path) = @_;
    return if !defined($path) || !-e $path;
    open my $fh, "<", $path or dief("%s: %s", $path, $!);
    my $lineno = 0;
    while (my $line = <$fh>) {
        ++$lineno;
        chomp $line;
        $line = trim_whitespace($line);
        next if $line eq "" || $line =~ /\A#/;

        my ($key, $value) = split /=/, $line, 2;
        dief("%s:%d: expected key=value", $path, $lineno) if !defined($value);
        $key = trim_whitespace($key);
        $value = trim_whitespace($value);
        if ($key eq "writable_dir" || $key eq "writable_path") {
            dief("%s:%d: %s requires a path", $path, $lineno, $key) if $value eq "";
            add_writable_path($dirs, $files, $seen, $value, $host_home, 0);
            next;
        }
        if ($key eq "optional_writable_dir" || $key eq "optional_writable_path") {
            dief("%s:%d: %s requires a path", $path, $lineno, $key) if $value eq "";
            add_writable_path($dirs, $files, $seen, $value, $host_home, 1);
            next;
        }
        dief("%s:%d: unknown key %s", $path, $lineno, $key);
    }
    close $fh or dief("%s: %s", $path, $!);
}

sub load_config_writable_paths {
    my ($dirs, $files, $seen, $host_home, $xdg_config_home) = @_;
    for my $path (global_config_paths()) {
        load_config_writable_paths_from_path($dirs, $files, $seen, $host_home, $path);
    }
    load_config_writable_paths_from_path($dirs, $files, $seen, $host_home, config_path($host_home, $xdg_config_home));
}

sub valid_env_name {
    my ($name) = @_;
    return defined($name) && $name =~ /\A[A-Za-z_][A-Za-z0-9_]*\z/;
}

sub unsetenv_name_is_reserved {
    my ($name) = @_;
    return 1 if $name =~ /\ASBBASH_/;
    my %reserved = map { $_ => 1 } qw(
        PATH
        PWD
        HOME
        TMPDIR
        HISTFILE
        SHELL
        SBRUN_ACTIVE
        USER
        LOGNAME
        TERM
        LANG
        LC_ALL
        LC_CTYPE
        BASH_SILENCE_DEPRECATION_WARNING
    );
    return $reserved{$name} ? 1 : 0;
}

sub parse_cli_options {
    my ($dirs, $files, $seen, $envdir_vars, $unsetenv_vars, $host_home, @argv) = @_;
    my $force_command = 0;
    my $i = 0;

    while ($i < @argv) {
        if ($argv[$i] eq "--") {
            $force_command = 1;
            ++$i;
            last;
        }
        if ($argv[$i] eq "-w" || $argv[$i] eq "--writable") {
            dief("%s requires a path argument", $argv[$i]) if $i + 1 >= @argv;
            add_writable_path($dirs, $files, $seen, $argv[$i + 1], $host_home, 0);
            $i += 2;
            next;
        }
        if ($argv[$i] eq "-e" || $argv[$i] eq "--envdir") {
            dief("%s requires an environment variable name", $argv[$i]) if $i + 1 >= @argv;
            dief("invalid envdir variable name %s", $argv[$i + 1]) if !valid_env_name($argv[$i + 1]);
            dief("cannot use --envdir and --unsetenv for the same variable %s", $argv[$i + 1])
                if grep { $_ eq $argv[$i + 1] } @$unsetenv_vars;
            push @$envdir_vars, $argv[$i + 1] if !grep { $_ eq $argv[$i + 1] } @$envdir_vars;
            $i += 2;
            next;
        }
        if ($argv[$i] eq "-v" || $argv[$i] eq "--unsetenv") {
            dief("%s requires an environment variable name", $argv[$i]) if $i + 1 >= @argv;
            dief("invalid unsetenv variable name %s", $argv[$i + 1]) if !valid_env_name($argv[$i + 1]);
            dief("cannot unset reserved environment variable %s", $argv[$i + 1]) if unsetenv_name_is_reserved($argv[$i + 1]);
            dief("cannot use --envdir and --unsetenv for the same variable %s", $argv[$i + 1])
                if grep { $_ eq $argv[$i + 1] } @$envdir_vars;
            push @$unsetenv_vars, $argv[$i + 1] if !grep { $_ eq $argv[$i + 1] } @$unsetenv_vars;
            $i += 2;
            next;
        }
        if ($argv[$i] =~ /\A--writable=(.*)\z/s) {
            my $value = $1;
            dief("--writable requires a path argument") if $value eq "";
            add_writable_path($dirs, $files, $seen, $value, $host_home, 0);
            ++$i;
            next;
        }
        if ($argv[$i] =~ /\A--envdir=(.*)\z/s) {
            my $value = $1;
            dief("--envdir requires an environment variable name") if $value eq "";
            dief("invalid envdir variable name %s", $value) if !valid_env_name($value);
            dief("cannot use --envdir and --unsetenv for the same variable %s", $value)
                if grep { $_ eq $value } @$unsetenv_vars;
            push @$envdir_vars, $value if !grep { $_ eq $value } @$envdir_vars;
            ++$i;
            next;
        }
        if ($argv[$i] =~ /\A--unsetenv=(.*)\z/s) {
            my $value = $1;
            dief("--unsetenv requires an environment variable name") if $value eq "";
            dief("invalid unsetenv variable name %s", $value) if !valid_env_name($value);
            dief("cannot unset reserved environment variable %s", $value) if unsetenv_name_is_reserved($value);
            dief("cannot use --envdir and --unsetenv for the same variable %s", $value)
                if grep { $_ eq $value } @$envdir_vars;
            push @$unsetenv_vars, $value if !grep { $_ eq $value } @$unsetenv_vars;
            ++$i;
            next;
        }
        last;
    }

    my @remaining = @argv[$i .. $#argv];
    return ($force_command, @remaining);
}

sub close_extra_fds {
    my $maxfd = sysconf(_SC_OPEN_MAX);
    $maxfd = 65536 if !defined($maxfd) || $maxfd <= 0 || $maxfd > 65536;
    for (my $fd = 3; $fd < $maxfd; ++$fd) {
        POSIX::close($fd);
    }
}

sub ensure_real_directory {
    my ($path) = @_;
    my @st = lstat($path);
    if (@st) {
        dief("%s exists and is a symlink; refusing to use it", $path) if -l _;
        dief("%s exists and is not a directory", $path) if !-d _;
        return;
    }
    dief("%s: %s", $path, $!) if $! != ENOENT;
    return if mkdir($path, 0700);
    if ($! != EEXIST) {
        dief("%s: %s", $path, $!);
    }
    @st = lstat($path);
    dief("%s: %s", $path, $!) if !@st;
    dief("%s exists and is not a real directory", $path) if -l _ || !-d _;
}

sub path_is_within {
    my ($path, $dir) = @_;
    return 0 if substr($path, 0, length($dir)) ne $dir;
    my $tail = substr($path, length($dir), 1);
    return $tail eq "" || $tail eq "/";
}

sub path_is_allowed_write_target {
    my ($path, $workdir, $extra_writable_dirs, $extra_writable_files) = @_;
    return 1 if path_is_within($path, $workdir);
    for my $dir (@$extra_writable_dirs) {
        return 1 if path_is_within($path, $dir);
    }
    for my $file (@$extra_writable_files) {
        return 1 if $path eq $file;
    }
    return 0;
}

sub realpath_if_possible {
    my ($path) = @_;
    my $resolved = realpath($path);
    return defined($resolved) ? $resolved : $path;
}

sub fd_regular_path {
    my ($fd) = @_;
    my $fh;

    open($fh, "<&=$fd") or open($fh, ">&=$fd") or return undef;
    my @st = stat($fh);
    return undef if !@st || !-f _;

    my $buf = "\0" x 4096;
    my $ok = fcntl($fh, F_GETPATH, $buf);
    dief("fd %d is redirected to a regular file outside the sandbox check path; refusing to start", $fd)
        if !$ok;
    $buf =~ s/\0.*\z//s;
    dief("fd %d is redirected to a regular file outside the sandbox check path; refusing to start", $fd)
        if $buf eq "";
    return $buf;
}

sub refuse_redirected_regular_stdio {
    my ($workdir, $extra_writable_dirs, $extra_writable_files) = @_;
    return if defined($ENV{SBBASH_ALLOW_STDIO_REDIRECTS}) && $ENV{SBBASH_ALLOW_STDIO_REDIRECTS} eq "1";

    for my $fd (1, 2) {
        my $raw_path = fd_regular_path($fd);
        next if !defined($raw_path);
        my $final_path = realpath_if_possible($raw_path);
        dief(
            "fd %d is redirected to %s outside allowed writable paths; refusing to start (set SBBASH_ALLOW_STDIO_REDIRECTS=1 to override)",
            $fd,
            $final_path
        ) if !path_is_allowed_write_target($final_path, $workdir, $extra_writable_dirs, $extra_writable_files);
    }
}

sub sanitize_env {
    my ($workdir, $tmpdir, $histfile, $host_home, $user_name, $shell_path, $envdir_root, $envdir_vars, $unsetenv_vars) = @_;
    my $term = $ENV{TERM};
    my $lang = $ENV{LANG};
    my $lc_all = $ENV{LC_ALL};
    my $lc_ctype = $ENV{LC_CTYPE};
    my $path = $ENV{PATH};

    delete @ENV{qw(BASH_ENV ENV DYLD_INSERT_LIBRARIES DYLD_LIBRARY_PATH DYLD_FRAMEWORK_PATH LD_LIBRARY_PATH)};

    $path = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        if !defined($path) || $path eq "";

    $ENV{PATH} = $path;
    $ENV{PWD} = $workdir;
    $ENV{HOME} = $host_home if defined($host_home) && $host_home ne "";
    $ENV{TMPDIR} = $tmpdir;
    if (defined($histfile)) {
        $ENV{HISTFILE} = $histfile;
    } else {
        delete $ENV{HISTFILE};
    }
    $ENV{SHELL} = $shell_path;
    $ENV{SBBASH_WORKDIR} = $workdir;
    $ENV{SBRUN_ACTIVE} = "1";

    if (shell_is_bash($shell_path)) {
        $ENV{BASH_SILENCE_DEPRECATION_WARNING} = "1";
    } else {
        delete $ENV{BASH_SILENCE_DEPRECATION_WARNING};
    }

    $ENV{SBBASH_HOST_HOME} = $host_home if defined($host_home) && $host_home ne "";
    if (defined($user_name) && $user_name ne "") {
        $ENV{USER} = $user_name;
        $ENV{LOGNAME} = $user_name;
    }
    $ENV{TERM} = $term if defined($term) && $term ne "";
    $ENV{LANG} = $lang if defined($lang) && $lang ne "";
    $ENV{LC_ALL} = $lc_all if defined($lc_all) && $lc_all ne "";
    $ENV{LC_CTYPE} = $lc_ctype if defined($lc_ctype) && $lc_ctype ne "";

    if (defined($envdir_root)) {
        for my $name (@$envdir_vars) {
            $ENV{$name} = path_join($envdir_root, $name);
        }
    }

    delete @ENV{@$unsetenv_vars} if @$unsetenv_vars;
}

sub escape_sandbox_string {
    my ($s) = @_;
    dief("path contains a newline: %s", $s) if $s =~ /[\r\n]/;
    $s =~ s/\\/\\\\/g;
    $s =~ s/"/\\"/g;
    return $s;
}

sub build_profile_text {
    my ($extra_writable_dirs, $extra_writable_files, $allow_histfile) = @_;
    my $profile = <<'EOF';
(version 1)
(deny default)
(import "system.sb")

; behave like a normal shell, but only allow writes inside WORKDIR
(allow process*)
(allow network*)
(allow sysctl-read)
(allow file-read*)

; common special files and tty ioctls for interactive tools
(allow file-read-data
    (literal "/dev/random")
    (literal "/dev/urandom"))
(allow file-read-data file-write-data file-ioctl
    (literal "/dev/null")
    (literal "/dev/tty")
    (literal (param "TTY")))

; the writable places are rooted under the launch directory and any configured extras
(allow file-write*
    (subpath (param "WORKDIR"))
EOF

    for my $dir (@$extra_writable_dirs) {
        $profile .= sprintf("    (subpath \"%s\")\n", escape_sandbox_string($dir));
    }
    $profile .= ")\n";
    if ($allow_histfile || @$extra_writable_files) {
        $profile .= "(allow file-write*\n";
        $profile .= "    (literal (param \"HISTFILE\"))\n" if $allow_histfile;
        for my $file (@$extra_writable_files) {
            $profile .= sprintf("    (literal \"%s\")\n", escape_sandbox_string($file));
        }
        $profile .= ")\n";
    }
    return $profile;
}

sub exec_sandbox {
    my (@argv) = @_;
    exec(@argv) or die_errno("exec(/usr/bin/sandbox-exec)");
}

my @pw = getpwuid($<);
my $user_name = defined($pw[0]) ? $pw[0] : undef;
my $host_home = defined($pw[7]) ? $pw[7] : undef;
my $pw_shell = defined($pw[8]) ? $pw[8] : undef;
my $shell_path = pick_shell($ENV{SHELL}, $pw_shell);
my $xdg_config_home = $ENV{XDG_CONFIG_HOME};
ensure_user_config_exists($host_home, $xdg_config_home);

if (cli_requests_help(@ARGV)) {
    print_help(*STDOUT);
    exit 0;
}
if (cli_requests_version(@ARGV)) {
    print_version(*STDOUT);
    exit 0;
}

-x "/usr/bin/sandbox-exec" or dief("/usr/bin/sandbox-exec is unavailable on this macOS installation");

my $workdir = realpath(".");
die_errno("realpath(.)") if !defined($workdir);
chdir($workdir) or die_errno("chdir($workdir)");

my @extra_writable_dirs;
my @extra_writable_files;
my %seen_extra_writable_paths;
my @envdir_vars;
my @unsetenv_vars;
load_config_writable_paths(\@extra_writable_dirs, \@extra_writable_files, \%seen_extra_writable_paths, $host_home, $xdg_config_home);
my ($force_command, @remaining_argv) =
    parse_cli_options(\@extra_writable_dirs, \@extra_writable_files, \%seen_extra_writable_paths, \@envdir_vars, \@unsetenv_vars, $host_home, @ARGV);

refuse_redirected_regular_stdio($workdir, \@extra_writable_dirs, \@extra_writable_files);

my $tmpdir = "/tmp";
my $histfile = defined($host_home) && $host_home ne ""
    ? path_join($host_home, history_file_name_for_shell($shell_path))
    : undef;

my $envdir_root;
if (@envdir_vars) {
    $envdir_root = path_join($workdir, ".sbrun");
    ensure_real_directory($envdir_root);
    for my $name (@envdir_vars) {
        ensure_real_directory(path_join($envdir_root, $name));
    }
}

sanitize_env($workdir, $tmpdir, $histfile, $host_home, $user_name, $shell_path, $envdir_root, \@envdir_vars, \@unsetenv_vars);
close_extra_fds();

my $tty_path = ttyname(fileno(STDIN));
$tty_path = ttyname(fileno(STDOUT)) if !defined($tty_path);
$tty_path = ttyname(fileno(STDERR)) if !defined($tty_path);
$tty_path = "/dev/tty" if !defined($tty_path) || $tty_path eq "";

my $profile = build_profile_text(\@extra_writable_dirs, \@extra_writable_files, defined($histfile));

dief("`--` requires a command to run") if $force_command && !@remaining_argv;

my @run_argv;
if (!@remaining_argv) {
    @run_argv = ($shell_path, "-l", "-i");
} elsif (!$force_command && $remaining_argv[0] =~ /\A-/) {
    @run_argv = ($shell_path, @remaining_argv);
} else {
    @run_argv = @remaining_argv;
}

exec_sandbox(
    "/usr/bin/sandbox-exec",
    "-D", "WORKDIR=$workdir",
    "-D", "TTY=$tty_path",
    (defined($histfile) ? ("-D", "HISTFILE=$histfile") : ()),
    "-p", $profile,
    @run_argv,
);
