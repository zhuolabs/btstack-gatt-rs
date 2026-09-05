use btstack_gatt::Error;
use btstack_nusb::UsbDeviceSelector;

pub const USAGE: &str = "Usage: gatt-peripheral [--vid HEX --pid HEX] [--seconds N] [--probe]
    --vid HEX      USB vendor ID (hexadecimal, with or without 0x)
    --pid HEX      USB product ID (--did is an alias)
    --seconds N    Stop after N seconds of advertising
    --probe        Open the device, print endpoints, and exit
    --help, -h     Show this help without accessing USB
Defaults to VID 0411 / PID 0374 when neither ID is supplied.
Android apps must pass a UsbManager FD to the library's from_fd API.";

#[derive(Debug)]
pub struct Args {
    pub selector: UsbDeviceSelector,
    pub seconds: Option<u64>,
    pub probe: bool,
    pub help: bool,
}

impl Args {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, Error> {
        let mut args = args.into_iter();
        let (mut vid, mut pid, mut seconds) = (None, None, None);
        let (mut probe, mut help) = (false, false);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--vid" | "--pid" | "--did" => {
                    let slot = if arg == "--vid" { &mut vid } else { &mut pid };
                    if slot.is_some() {
                        return Err(format!("Duplicate USB ID option: {arg}").into());
                    }
                    let value = args
                        .next()
                        .ok_or_else(|| format!("Missing value for {arg}"))?;
                    *slot = Some(parse_usb_id(&value)?);
                }
                "--seconds" => {
                    if seconds.is_some() {
                        return Err("Duplicate --seconds option".into());
                    }
                    let value = args.next().ok_or("Missing value for --seconds")?;
                    seconds = Some(
                        value
                            .parse()
                            .map_err(|_| "--seconds must be a nonnegative integer")?,
                    );
                }
                "--probe" => probe = true,
                "--help" | "-h" => help = true,
                _ => return Err(format!("Unknown argument: {arg}\n{USAGE}").into()),
            }
        }
        let selector = match (vid, pid) {
            (Some(vid), Some(pid)) => UsbDeviceSelector::new(vid, pid),
            (None, None) => UsbDeviceSelector::new(0x0411, 0x0374),
            _ => return Err("Specify both --vid and --pid (or --did)".into()),
        };
        Ok(Self {
            selector,
            seconds,
            probe,
            help,
        })
    }
}

fn parse_usb_id(value: &str) -> Result<u16, Error> {
    let digits = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if digits.is_empty() || digits.len() > 4 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("Invalid USB ID '{value}'; expected 1 to 4 hexadecimal digits").into());
    }
    Ok(u16::from_str_radix(digits, 16)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Args, Error> {
        Args::parse(args.iter().map(|s| s.to_string()))
    }
    #[test]
    fn accepts_hex_ids_and_did_alias() {
        let args = parse(&[
            "--vid",
            "0x0411",
            "--did",
            "0374",
            "--seconds",
            "2",
            "--probe",
        ])
        .unwrap();
        assert_eq!(args.selector.vendor_id, 0x0411);
        assert_eq!(args.selector.product_id, 0x0374);
        assert_eq!(args.seconds, Some(2));
        assert!(args.probe);
        assert_eq!(
            parse(&["--vid", "0BDA", "--pid", "0XFFFF"])
                .unwrap()
                .selector
                .product_id,
            0xffff
        );
    }
    #[test]
    fn rejects_partial_duplicate_and_invalid_ids() {
        for args in [
            vec!["--vid", "0411"],
            vec!["--pid", "0374"],
            vec!["--vid"],
            vec!["--vid", "-1", "--pid", "0374"],
            vec!["--vid", "10000", "--pid", "0374"],
            vec!["--vid", "+1", "--pid", "0374"],
            vec!["--vid", "0x", "--pid", "0374"],
            vec!["--vid", "0411", "--pid", "0374", "--did", "0374"],
            vec!["--seconds"],
            vec!["--seconds", "-1"],
            vec!["--unknown"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }
    #[test]
    fn preserves_defaults_and_parses_help() {
        let args = parse(&[]).unwrap();
        assert_eq!(args.selector.vendor_id, 0x0411);
        assert_eq!(args.selector.product_id, 0x0374);
        assert!(args.seconds.is_none());
        assert!(parse(&["--help"]).unwrap().help);
    }
}
