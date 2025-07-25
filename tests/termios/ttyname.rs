use rustix::io;
use rustix::termios::{isatty, ttyname};
use std::fs::File;

#[test]
fn test_ttyname_ok() {
    let file = match File::open("/dev/stdin") {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(err) => Err(err).unwrap(),
    };
    if isatty(&file) {
        assert!(ttyname(&file, Vec::new())
            .unwrap()
            .into_string()
            .unwrap()
            .starts_with("/dev/"));
    }
}

#[test]
fn test_ttyname_not_tty() {
    let file = File::open("Cargo.toml").unwrap();
    assert_eq!(ttyname(&file, Vec::new()).unwrap_err(), io::Errno::NOTTY);

    let file = File::open("/dev/null").unwrap();
    assert_eq!(ttyname(&file, Vec::new()).unwrap_err(), io::Errno::NOTTY);
}
