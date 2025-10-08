# Image Janitor

Image Janitor is a command-line tool for cleaning up unused kernel drivers and firmware from a Linux system. It helps to reduce the size of a Linux image by removing unnecessary files.

## Features

*   **Driver Cleanup**: Removes unused kernel drivers.
*   **Firmware Cleanup**: Removes unused firmware files.
*   **Configuration**: Uses configuration files to determine which files to keep and which to delete.
*   **Dependency Resolution**: Resolves dependencies between kernel modules to avoid breaking the system.

## Usage

### Driver Cleanup

To clean up unused kernel drivers, run the following command:

```bash
image-janitor driver-cleanup
```

By default, the command will perform a dry run and only show the files that would be deleted. To actually delete the files, use the `--delete` flag:

```bash
image-janitor driver-cleanup --delete
```

You can also specify the directory containing the kernel modules and the configuration files to use:

```bash
image-janitor driver-cleanup --module-dir /path/to/modules --config-files /path/to/config1,/path/to/config2
```

### Firmware Cleanup

To clean up unused firmware, run the following command:

```bash
image-janitor fw-cleanup
```

By default, the command will perform a dry run and only show the files that would be deleted. To actually delete the files, use the `--delete` flag:

```bash
image-janitor fw-cleanup --delete
```

You can also specify the directory containing the kernel modules and the firmware files:

```bash
image-janitor fw-cleanup --module-dir /path/to/modules --firmware-dir /path/to/firmware
```

## Building from Source

To build the project from source, you will need to have Rust installed. You can then clone the repository and build the project using Cargo:

```bash
git clone https://github.com/example/image-janitor.git
cd image-janitor
cargo build --release
```

The executable will be located in the `target/release` directory.

## Configuration

The configuration files use a simple format. Each line contains a regular expression that is matched against the path of a file. If the path matches a regular expression, the file is kept. If the path does not match any regular expression, the file is deleted.

You can also specify which files to delete by prefixing the regular expression with a `-`. For example, to delete all files in the `drivers/net` directory, you would add the following line to your configuration file:

```
-drivers/net/.*
```

The configuration files also support architecture-specific sections. For example, to specify that a driver should only be kept on x86_64 systems, you would add the following lines to your configuration file:

```
<x86_64>
drivers/net/ethernet/intel/.*
</x86_64>
```
