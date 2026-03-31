#!/usr/bin/perl

use strict;
use warnings;

use Cwd qw(realpath);
use Errno qw(EACCES ENOENT EEXIST EPERM);
use POSIX qw(_SC_OPEN_MAX sysconf ttyname);

use constant F_GETPATH => 50;

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
        "current directory tree plus any configured extra writable directories.\n",
        "\n",
        "Options:\n",
        "  -h, --help           Show this help and exit\n",
        "  -w, --writable DIR   Allow writes under DIR; may be repeated\n",
        "  --                   Stop parsing $prog_name options and force command mode\n",
        "\n",
        "Behavior:\n",
        "  With no command, start \$SHELL interactively.\n",
        "  If the first non-option argument starts with '-', pass arguments to \$SHELL.\n",
        "  Otherwise run the given command directly.\n",
        "\n",
        "Config:\n",
        "  \$XDG_CONFIG_HOME/sbbash/config or ~/.config/sbbash/config\n",
        "  Format: writable_dir=/absolute/or/~/path\n";
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
        if ($argv[$i] =~ /\A--writable=/) {
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
    return path_join(path_join($xdg_config_home, "sbbash"), "config")
        if defined($xdg_config_home) && $xdg_config_home =~ m{\A/};
    return path_join(path_join(path_join($host_home, ".config"), "sbbash"), "config")
        if defined($host_home) && $host_home ne "";
    return undef;
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

sub resolve_writable_dir {
    my ($path, $host_home) = @_;
    my $expanded = expand_home_path($path, $host_home);
    my $resolved = realpath($expanded);
    dief("additional writable directory %s: %s", $path, $!)
        if !defined($resolved);
    dief("additional writable directory %s resolves to %s, which is not a directory", $path, $resolved)
        if !-d $resolved;
    return $resolved;
}

sub add_writable_dir {
    my ($dirs, $seen, $path, $host_home) = @_;
    my $resolved = resolve_writable_dir($path, $host_home);
    return if $seen->{$resolved};
    push @$dirs, $resolved;
    $seen->{$resolved} = 1;
}

sub load_config_writable_dirs {
    my ($dirs, $seen, $host_home, $xdg_config_home) = @_;
    my $path = config_path($host_home, $xdg_config_home);
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
        dief("%s:%d: unknown key %s", $path, $lineno, $key) if $key ne "writable_dir";
        dief("%s:%d: writable_dir requires a path", $path, $lineno) if $value eq "";
        add_writable_dir($dirs, $seen, $value, $host_home);
    }
    close $fh or dief("%s: %s", $path, $!);
}

sub parse_cli_options {
    my ($dirs, $seen, $host_home, @argv) = @_;
    my $force_command = 0;
    my $i = 0;

    while ($i < @argv) {
        if ($argv[$i] eq "--") {
            $force_command = 1;
            ++$i;
            last;
        }
        if ($argv[$i] eq "-w" || $argv[$i] eq "--writable") {
            dief("%s requires a directory argument", $argv[$i]) if $i + 1 >= @argv;
            add_writable_dir($dirs, $seen, $argv[$i + 1], $host_home);
            $i += 2;
            next;
        }
        if ($argv[$i] =~ /\A--writable=(.*)\z/s) {
            my $value = $1;
            dief("--writable requires a directory argument") if $value eq "";
            add_writable_dir($dirs, $seen, $value, $host_home);
            ++$i;
            next;
        }
        last;
    }

    my @remaining = @argv[$i .. $#argv];
    return ($force_command, @remaining);
}

sub try_ensure_dir {
    my ($path, $mode) = @_;

    if (lstat($path)) {
        dief("%s exists but is a symlink", $path) if -l _;
        dief("%s exists but is not a directory", $path) if !-d _;
        return 1;
    }
    return 0 if $! == EACCES || $! == EPERM;
    die_errno("lstat($path)") if $! != ENOENT;

    return 1 if mkdir($path, $mode);

    if ($! == EEXIST) {
        if (lstat($path)) {
            dief("%s exists but is a symlink", $path) if -l _;
            return 1 if -d _;
            dief("%s exists but is not a directory", $path);
        }
    }
    return 0 if $! == EACCES || $! == EPERM;
    die_errno("mkdir($path)");
}

sub close_extra_fds {
    my $maxfd = sysconf(_SC_OPEN_MAX);
    $maxfd = 65536 if !defined($maxfd) || $maxfd <= 0 || $maxfd > 65536;
    for (my $fd = 3; $fd < $maxfd; ++$fd) {
        POSIX::close($fd);
    }
}

sub path_is_within {
    my ($path, $dir) = @_;
    return 0 if substr($path, 0, length($dir)) ne $dir;
    my $tail = substr($path, length($dir), 1);
    return $tail eq "" || $tail eq "/";
}

sub path_is_within_allowed_dirs {
    my ($path, $workdir, $extra_writable_dirs) = @_;
    return 1 if path_is_within($path, $workdir);
    for my $dir (@$extra_writable_dirs) {
        return 1 if path_is_within($path, $dir);
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
    my ($workdir, $extra_writable_dirs) = @_;
    return if defined($ENV{SBBASH_ALLOW_STDIO_REDIRECTS}) && $ENV{SBBASH_ALLOW_STDIO_REDIRECTS} eq "1";

    for my $fd (1, 2) {
        my $raw_path = fd_regular_path($fd);
        next if !defined($raw_path);
        my $final_path = realpath_if_possible($raw_path);
        dief(
            "fd %d is redirected to %s outside allowed writable directories; refusing to start (set SBBASH_ALLOW_STDIO_REDIRECTS=1 to override)",
            $fd,
            $final_path
        ) if !path_is_within_allowed_dirs($final_path, $workdir, $extra_writable_dirs);
    }
}

sub sanitize_env {
    my ($workdir, $sandbox_home, $sandbox_tmp, $histfile, $host_home, $user_name, $shell_path) = @_;
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
    $ENV{HOME} = $sandbox_home;
    $ENV{TMPDIR} = $sandbox_tmp;
    $ENV{HISTFILE} = $histfile;
    $ENV{SHELL} = $shell_path;
    $ENV{SBBASH_WORKDIR} = $workdir;

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
}

sub escape_sandbox_string {
    my ($s) = @_;
    dief("path contains a newline: %s", $s) if $s =~ /[\r\n]/;
    $s =~ s/\\/\\\\/g;
    $s =~ s/"/\\"/g;
    return $s;
}

sub build_profile_text {
    my ($extra_writable_dirs) = @_;
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

; the writable places are rooted under the launch directory plus any configured extras
(allow file-write*
    (subpath (param "WORKDIR"))
    (subpath (param "HOME"))
    (subpath (param "TMPDIR"))
EOF

    for my $dir (@$extra_writable_dirs) {
        $profile .= sprintf("    (subpath \"%s\")\n", escape_sandbox_string($dir));
    }
    $profile .= ")\n";
    return $profile;
}

sub exec_sandbox {
    my (@argv) = @_;
    exec(@argv) or die_errno("exec(/usr/bin/sandbox-exec)");
}

if (cli_requests_help(@ARGV)) {
    print_help(*STDOUT);
    exit 0;
}

my @pw = getpwuid($<);
my $user_name = defined($pw[0]) ? $pw[0] : undef;
my $host_home = defined($pw[7]) ? $pw[7] : undef;
my $pw_shell = defined($pw[8]) ? $pw[8] : undef;
my $shell_path = pick_shell($ENV{SHELL}, $pw_shell);
my $xdg_config_home = $ENV{XDG_CONFIG_HOME};

-x "/usr/bin/sandbox-exec" or dief("/usr/bin/sandbox-exec is unavailable on this macOS installation");

my $workdir = realpath(".");
die_errno("realpath(.)") if !defined($workdir);
chdir($workdir) or die_errno("chdir($workdir)");

my @extra_writable_dirs;
my %seen_extra_writable_dirs;
load_config_writable_dirs(\@extra_writable_dirs, \%seen_extra_writable_dirs, $host_home, $xdg_config_home);
my ($force_command, @remaining_argv) =
    parse_cli_options(\@extra_writable_dirs, \%seen_extra_writable_dirs, $host_home, @ARGV);

refuse_redirected_regular_stdio($workdir, \@extra_writable_dirs);

my $sandbox_home = path_join($workdir, ".sbbash-home");
my $sandbox_tmp = path_join($workdir, ".sbbash-tmp");
$sandbox_home = $workdir if !try_ensure_dir($sandbox_home, 0700);
$sandbox_tmp = $workdir if !try_ensure_dir($sandbox_tmp, 0700);

my $histname = history_file_name_for_shell($shell_path);
my $histfile = $sandbox_home eq $workdir ? path_join($workdir, ".sbbash_history") : path_join($sandbox_home, $histname);

sanitize_env($workdir, $sandbox_home, $sandbox_tmp, $histfile, $host_home, $user_name, $shell_path);
close_extra_fds();

my $tty_path = ttyname(fileno(STDIN));
$tty_path = ttyname(fileno(STDOUT)) if !defined($tty_path);
$tty_path = ttyname(fileno(STDERR)) if !defined($tty_path);
$tty_path = "/dev/tty" if !defined($tty_path) || $tty_path eq "";

my $profile = build_profile_text(\@extra_writable_dirs);

dief("`--` requires a command to run") if $force_command && !@remaining_argv;

my @run_argv;
if (!@remaining_argv) {
    @run_argv = ($shell_path, "-i");
} elsif (!$force_command && $remaining_argv[0] =~ /\A-/) {
    @run_argv = ($shell_path, @remaining_argv);
} else {
    @run_argv = @remaining_argv;
}

exec_sandbox(
    "/usr/bin/sandbox-exec",
    "-D", "WORKDIR=$workdir",
    "-D", "HOME=$sandbox_home",
    "-D", "TMPDIR=$sandbox_tmp",
    "-D", "TTY=$tty_path",
    "-p", $profile,
    @run_argv,
);
