// opcodes we want to test
// not branching, not in, not out
struct Opcode(u8, usize);

const OPCODES_TO_TEST: [Opcode; 181] = [
    Opcode(0x00, 0),
    Opcode(0x01, 0),
    Opcode(0x02, 0),
    Opcode(0x03, 0),
    Opcode(0x04, 0),
    Opcode(0x05, 0),
    Opcode(0x06, 0),
    Opcode(0x07, 0),
    Opcode(0x08, 0),
    Opcode(0x09, 0),
    Opcode(0x0a, 0),
    Opcode(0x0b, 0),
    Opcode(0x0e, 0),
    Opcode(0x0f, 0),
    Opcode(0x10, 0),
    Opcode(0x11, 0),
    Opcode(0x12, 0),
    Opcode(0x13, 0),
    Opcode(0x14, 0),
    Opcode(0x15, 0),
    Opcode(0x16, 0),
    // Opcode(0x17, 0), // no write
    Opcode(0x18, 0),
    Opcode(0x19, 0),
    Opcode(0x1a, 0),
    Opcode(0x1b, 0),
    Opcode(0x1d, 0),
    Opcode(0x1e, 0),
    Opcode(0x1f, 0),
    Opcode(0x20, 1),
    Opcode(0x21, 1),
    Opcode(0x22, 1),
    Opcode(0x23, 1),
    Opcode(0x24, 1),
    Opcode(0x25, 1),
    Opcode(0x2b, 0),
    Opcode(0x2c, 0),
    Opcode(0x30, 0),
    Opcode(0x31, 0),
    Opcode(0x32, 0),
    Opcode(0x33, 0),
    Opcode(0x34, 0),
    Opcode(0x35, 0),
    Opcode(0x36, 0),
    Opcode(0x37, 0),
    Opcode(0x38, 0),
    Opcode(0x39, 0),
    Opcode(0x3a, 0),
    Opcode(0x3b, 0),
    Opcode(0x3c, 0),
    Opcode(0x3d, 0),
    Opcode(0x3e, 0),
    Opcode(0x40, 0),
    Opcode(0x41, 0),
    Opcode(0x42, 0),
    Opcode(0x43, 0),
    Opcode(0x44, 0),
    Opcode(0x45, 0),
    Opcode(0x46, 0),
    Opcode(0x47, 0),
    Opcode(0x48, 0),
    Opcode(0x49, 0),
    Opcode(0x4a, 0),
    Opcode(0x4b, 0),
    Opcode(0x4c, 0),
    Opcode(0x4d, 0),
    Opcode(0x4e, 0),
    Opcode(0x50, 0),
    Opcode(0x51, 0),
    Opcode(0x52, 0),
    Opcode(0x53, 0),
    Opcode(0x54, 0),
    Opcode(0x55, 0),
    Opcode(0x56, 0),
    Opcode(0x57, 0),
    Opcode(0x58, 0),
    Opcode(0x59, 0),
    Opcode(0x5a, 0),
    Opcode(0x5b, 0),
    Opcode(0x5c, 0),
    Opcode(0x5d, 0),
    Opcode(0x5e, 0),
    Opcode(0x60, 0),
    Opcode(0x61, 0),
    Opcode(0x62, 0),
    Opcode(0x63, 0),
    Opcode(0x64, 0),
    Opcode(0x65, 0),
    Opcode(0x66, 0),
    Opcode(0x67, 0),
    Opcode(0x68, 0),
    Opcode(0x69, 0),
    Opcode(0x6a, 0),
    Opcode(0x6b, 0),
    Opcode(0x6c, 0),
    Opcode(0x6d, 0),
    Opcode(0x6e, 0),
    Opcode(0x6f, 0),
    Opcode(0x70, 0),
    Opcode(0x71, 0),
    Opcode(0x72, 0),
    Opcode(0x73, 0),
    Opcode(0x74, 0),
    Opcode(0x75, 0),
    Opcode(0x76, 0),
    Opcode(0x77, 0),
    Opcode(0x78, 0),
    Opcode(0x79, 0),
    Opcode(0x7a, 0),
    Opcode(0x7b, 0),
    Opcode(0x7c, 0),
    Opcode(0x7d, 0),
    Opcode(0x7e, 0),
    Opcode(0x7f, 0),
    Opcode(0x88, 0),
    Opcode(0x89, 0),
    Opcode(0x8a, 0),
    Opcode(0x8b, 0),
    Opcode(0x8c, 0),
    Opcode(0x8d, 0),
    Opcode(0x8e, 0),
    Opcode(0xc0, 0),
    Opcode(0xc1, 0),
    Opcode(0xc2, 0),
    Opcode(0xc3, 0),
    Opcode(0xc4, 0),
    Opcode(0xc5, 0),
    Opcode(0xc6, 0),
    Opcode(0xc7, 0),
    Opcode(0xc8, 0),
    Opcode(0xc9, 0),
    Opcode(0xca, 0),
    Opcode(0xcb, 0),
    Opcode(0xcc, 0),
    Opcode(0xcd, 0),
    Opcode(0xce, 0),
    Opcode(0xd0, 0),
    Opcode(0xd1, 0),
    Opcode(0xd2, 0),
    Opcode(0xd3, 0),
    Opcode(0xd4, 0),
    Opcode(0xd5, 0),
    Opcode(0xd6, 0),
    Opcode(0xd7, 0),
    Opcode(0xd8, 0),
    Opcode(0xd9, 0),
    Opcode(0xda, 0),
    Opcode(0xdb, 0),
    Opcode(0xdc, 0),
    Opcode(0xdd, 0),
    Opcode(0xde, 0),
    Opcode(0xe0, 0),
    Opcode(0xe1, 0),
    Opcode(0xe1, 0),
    Opcode(0xe2, 0),
    Opcode(0xe3, 0),
    Opcode(0xe4, 0),
    Opcode(0xe5, 0),
    Opcode(0xe6, 0),
    Opcode(0xe7, 0),
    Opcode(0xe8, 0),
    Opcode(0xe9, 0),
    Opcode(0xea, 0),
    Opcode(0xeb, 0),
    Opcode(0xec, 0),
    Opcode(0xed, 0),
    Opcode(0xee, 0),
    Opcode(0xf0, 0),
    Opcode(0xf1, 0),
    Opcode(0xf2, 0),
    Opcode(0xf3, 0),
    Opcode(0xf4, 0),
    Opcode(0xf5, 0),
    Opcode(0xf6, 0),
    Opcode(0xf7, 0),
    Opcode(0xf8, 0),
    Opcode(0xf9, 0),
    Opcode(0xfa, 0),
    Opcode(0xfb, 0),
    Opcode(0xfc, 0),
    Opcode(0xfd, 0),
    Opcode(0xfe, 0),
];

// name the binary as maze.bin, use "mame -debug channelf maze"
// in mame debugger use this to trace
// bpset 1000
// trace test.log,0,noloop,{tracelog "A=%02X W=%02X IS=%02X R0=%02X R1=%02X R2=%02X R3=%02X R4=%02X ", a, w, is, r0,r1,r2,r3,r4}

fn main() {
    let mut binary = [0u8; 2 * 1024];

    for i in 0..binary.len() {
        binary[i] = OPCODES_TO_TEST[fastrand::usize(..OPCODES_TO_TEST.len())].0;
    }

    let mut i = 2usize;
    loop {
        let opcode = &OPCODES_TO_TEST[fastrand::usize(..OPCODES_TO_TEST.len())];

        if i + 1 + opcode.1 <= binary.len() {
            binary[i] = opcode.0;
            i += 1;

            for _ in 0..opcode.1 {
                binary[i] = fastrand::u8(0..255);
                i += 1;
            }
        } else {
            continue;
        }

        if i >= binary.len() {
            break;
        }
    }

    binary[0] = 0x55;
    binary[1] = 0x00;

    std::fs::write("./test.bin", &binary).unwrap();
}
