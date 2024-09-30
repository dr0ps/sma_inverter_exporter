use socket2::{SockAddr, Socket};
use bytebuffer_new::ByteBuffer;
use std::borrow::BorrowMut;
use std::mem::MaybeUninit;
use bytebuffer_new::Endian::{BigEndian, LittleEndian};
use std::time::{SystemTime, UNIX_EPOCH};
use crate::inverter::Lri::{BatChaStt, BatTmpVal, BatAmp, BatVol, DcMsVol, DcMsAmp, MeteringTotWhOut, MeteringDyWhOut};
use std::net::SocketAddr;

#[derive(Clone)]
pub struct Inverter {
    pub address: SocketAddr,
    packet_id : u32,
    susy_id : u16,
    serial : u32,
}

pub struct InverterError {
    pub message : &'static str,
}

pub struct DataType {
    command: u32,
    first: u32,
    last: u32,
}

const fn gen_susy_id() -> u16 {
    125
}

fn gen_serial() -> u32 {
    900000000 + rand::random::<u32>() % 100000000
}

pub enum Lri {
    BatChaStt = 0x00295A00,   // *00* Current battery charge status
    DcMsVol = 0x00451F00,   // *40* DC voltage input (aka SPOT_UDC1 / SPOT_UDC2)
    DcMsAmp = 0x00452100,   // *40* DC current input (aka SPOT_IDC1 / SPOT_IDC2)
    BatTmpVal = 0x00495B00,   // *40* Battery temperature
    BatVol = 0x00495C00,   // *40* Battery voltage
    BatAmp = 0x00495D00,   // *40* Battery current
    MeteringTotWhOut = 0x00260100,   // *00* Total yield (aka SPOT_ETOTAL)
    MeteringDyWhOut = 0x00262200,   // *00* Day yield (aka SPOT_ETODAY)
}

pub struct BatteryInfo {
    pub temperature: [u16;3],
    pub voltage: [u16;3],
    pub current: [i16;3]
}

pub struct DCInfo {
    pub voltage: [u16;2],
    pub current: [u16;2],
}

pub struct EnergyProductionInfo {
    pub daily_wh: u32,
    pub total_wh: u32,
}

impl Inverter {
    pub fn new(address : SocketAddr) -> Self {
        Self { address, packet_id:0, susy_id:gen_susy_id(), serial:gen_serial() }
    }

    fn write_packet(&mut self, buffer : &mut ByteBuffer, long_words : u8, control : u8, control_2 : u16)
    {
        self.packet_id += 1;
        buffer.write_u32(0x65601000);

        buffer.write_u8(long_words);
        buffer.write_u8(control);

        //SUSy id
        buffer.write_u16(0xffff);

        //Serial
        buffer.write_u32(0xffffffff);

        buffer.write_u16(control_2);

        buffer.write_u16(self.susy_id);

        buffer.write_u32(self.serial);

        buffer.write_u16(control_2);

        buffer.write_u16(0);
        buffer.write_u16(0);

        buffer.write_u16((self.packet_id | 0x8000) as u16);
    }

    fn write_packet_header(&mut self, buffer : &mut ByteBuffer) {
        buffer.write_u32(0x00414D53);  // SMA\0
        buffer.write_u32(0xA0020400);
        buffer.write_u32(0x01000000);

        buffer.write_u8(0);
        buffer.write_u8(0);
    }

    fn write_packet_length(&mut self, buffer : &mut ByteBuffer) {
        let data_length = (buffer.len() - 20) as u16;
        buffer.set_wpos(12);
        buffer.set_endian(BigEndian);
        buffer.write_u16(data_length);
    }

    /// Assume the `buf`fer to be initialised.
    // TODO: replace with `MaybeUninit::slice_assume_init_ref` once stable.
    unsafe fn assume_init(&mut self, buf: &[MaybeUninit<u8>]) -> &[u8] {
        &*(buf as *const [MaybeUninit<u8>] as *const [u8])
    }

    pub fn login(&mut self, socket:&Socket, password : &str) -> Result<u16, InverterError>{

        let mut buffer = ByteBuffer::new();
        buffer.set_endian(LittleEndian);

        self.write_packet_header(buffer.borrow_mut());

        self.write_packet(buffer.borrow_mut(), 0x0e, 0xa0, 0x0100);

        buffer.write_u32(0xFFFD040C);

        buffer.write_u32(0x07);
        buffer.write_u32(0x00000384);

        let start = SystemTime::now();
        let since_the_epoch = start
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards");

        buffer.write_u32(since_the_epoch.as_secs() as u32);
        buffer.write_u32(0);

        let enc_char = 0x88; //admin:0xbb
        let password_bytes = password.as_bytes();

        for byte in password_bytes {
            buffer.write_u8(byte + enc_char );
        }
        for _i in password_bytes.len()..12 {
            buffer.write_u8(enc_char );
        }

        buffer.write_u32(0);

        self.write_packet_length(buffer.borrow_mut());

        match socket.send_to(buffer.to_bytes().as_mut(), &SockAddr::from(self.address)) {
            Ok(_result) => {

            }
            Err(error) => {
                println!("{}", error);
            }
        }

        let mut buf = [MaybeUninit::new(0_u8); 500];
        match socket.recv_from(buf.as_mut()) {
            Ok((len, remote_addr)) => {
                if remote_addr.as_socket().unwrap().eq(&self.address) {
                    let mut buffer = ByteBuffer::from_bytes(unsafe { self.assume_init(&buf[0..len]) } );
                    buffer.set_endian(LittleEndian);
                    //L1
                    let l1_magic_number = buffer.read_u32();
                    if l1_magic_number != 0x00414D53 {
                        return Err(InverterError { message: "Packet does not start with SMA" });
                    }
                    buffer.read_u32();
                    buffer.read_u32();
                    let packet_length = buffer.read_u16();

                    //L2
                    let l2_magic_number = buffer.read_u32();
                    let _long_words = buffer.read_u8();
                    let _ctrl = buffer.read_u8();

                    if packet_length > 0 {
                        if l2_magic_number == 0x65601000 {
                            let _dest_susy_id = buffer.read_u16();
                            let _dest_serial = buffer.read_u32();
                            buffer.read_u16();

                            let _source_susy_id = buffer.read_u16();
                            let _source_serial = buffer.read_u32();
                            buffer.read_u16();

                            let error_code = buffer.read_u16();
                            let _fragment_id = buffer.read_u16();
                            let packet_id = buffer.read_u16();

                            if packet_id & 0x7FFF == self.packet_id as u16 {
                                if error_code == 0 {
                                    Ok(error_code)
                                } else {
                                    Err(InverterError { message: "Login failed." })
                                }
                            } else {
                                Err(InverterError { message: "Invalid packet id." })
                            }
                        } else {
                            Err(InverterError { message: "Magic bytes do not match." })
                        }
                    } else {
                        Err(InverterError { message: "Packet length is zero." })
                    }
                } else {
                    Err(InverterError { message: "Sent from wrong address." })
                }
            }
            Err(err) => {
                println!("{}", err);
                Err(InverterError { message: "error" })
            }
        }
    }

    pub fn logoff(&mut self, socket:&Socket){

        let mut buffer = ByteBuffer::new();
        buffer.set_endian(LittleEndian);

        self.write_packet_header(buffer.borrow_mut());

        self.write_packet(buffer.borrow_mut(), 0x08, 0xa0, 0x0300);

        buffer.write_u32(0xFFFD010E);
        buffer.write_u32(0xFFFFFFFF);

        buffer.write_u32(0);
        self.write_packet_length(buffer.borrow_mut());

        match socket.send_to(buffer.to_bytes().as_mut(), &SockAddr::from(self.address)) {
            Ok(_result) => {

            }
            Err(error) => {
                println!("{}", error);
            }
        }
    }

    const SPOT_DC_VOLTAGE: DataType = DataType{command: 0x53800200, first: 0x00451F00, last: 0x004521FF};
    const BATTERY_CHARGE_STATUS: DataType = DataType{command: 0x51000200, first: 0x00295A00, last: 0x00295AFF};
    const BATTERY_INFO: DataType = DataType{command: 0x51000200, first: 0x00491E00, last: 0x00495DFF};
    const ENERGY_PRODUCTION: DataType = DataType{command : 0x54000200, first: 0x00260100, last: 0x002622FF};

    fn get_data(&mut self, socket:&Socket, data_type:&DataType) -> Result<ByteBuffer, InverterError>{
        let mut buffer = ByteBuffer::new();
        buffer.set_endian(LittleEndian);

        self.write_packet_header(buffer.borrow_mut());
        self.write_packet(buffer.borrow_mut(), 0x09, 0xA0, 0);

        buffer.write_u32(data_type.command);
        buffer.write_u32(data_type.first);
        buffer.write_u32(data_type.last);

        buffer.write_u32(0);
        self.write_packet_length(buffer.borrow_mut());

        match socket.send_to(buffer.to_bytes().as_mut(), &SockAddr::from(self.address)) {
            Ok(_result) => {

            }
            Err(error) => {
                println!("{}", error);
            }
        }

        let mut buf = [MaybeUninit::new(0_u8); 1024];
        match socket.recv_from(buf.as_mut()) {
            Ok((len, remote_addr)) => {
                if remote_addr.as_socket().unwrap().eq(&self.address) {
                    let mut buffer = ByteBuffer::from_bytes(unsafe { self.assume_init(&buf[0..len]) } );
                    buffer.set_endian(LittleEndian);
                    //L1
                    let l1_magic_number = buffer.read_u32();
                    if l1_magic_number != 0x00414D53 {
                        return Err(InverterError { message: "Wrong magic number." });
                    }
                    buffer.read_u32();
                    buffer.read_u32();
                    let packet_length = buffer.read_u16();
                    //L2
                    let l2_magic_number = buffer.read_u32();
                    let _long_words = buffer.read_u8();
                    let _ctrl = buffer.read_u8();

                    if packet_length > 0 {
                        if l2_magic_number == 0x65601000 {
                            let _dest_susy_id = buffer.read_u16();
                            let _dest_serial = buffer.read_u32();
                            buffer.read_u16();

                            let _source_susy_id = buffer.read_u16();
                            let _source_serial = buffer.read_u32();
                            buffer.read_u16();

                            let error_code = buffer.read_u16();
                            let _fragment_id = buffer.read_u16();
                            let packet_id = buffer.read_u16();

                            if packet_id & 0x7FFF == self.packet_id as u16 {
                                if error_code == 0 {
                                    buffer.read_bytes(12);

                                    Ok(buffer)
                                }
                                else if error_code == 21 {
                                    Err(InverterError { message: "Unsupported" })
                                }
                                else {
                                    Err(InverterError { message: "Error code" })
                                }
                            }
                            else {
                                Err(InverterError { message: "Wrong packed id." })
                            }
                        }
                        else {
                            Err(InverterError { message: "Wrong magic number." })
                        }
                    }
                    else {
                        Err(InverterError { message: "Zero packet length." })
                    }
                }
                else {
                    Err(InverterError { message: "Wrong source address." })
                }
            }
            Err(err) => {
                println!("{}", err);
                Err(InverterError { message: "Error" })
            }
        }
    }

    pub fn get_battery_charge_status(&mut self, socket: &Socket) -> Result<[u8;3], InverterError>
    {
        match self.get_data(socket, &Inverter::BATTERY_CHARGE_STATUS) {
            Ok(mut buffer) => {

                let mut battery_charge:[u8;3] = [0,0,0];

                while buffer.len() > buffer.get_rpos() {
                    let code = buffer.read_u32();
                    if code == 0 {
                        return Ok(battery_charge);
                    }
                    let lri = code & 0x00FFFF00;
                    let _data_type = code >> 24;

                    if lri == BatChaStt as u32 && battery_charge[0] == 0 {
                        let _date = buffer.read_u32();
                        let value = buffer.read_u32();
                        battery_charge[0] = value as u8;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    } else if lri == BatChaStt as u32 && battery_charge[1] == 0 {
                        let _date = buffer.read_u32();
                        let value = buffer.read_u32();
                        battery_charge[1] = value as u8;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    } else if lri == BatChaStt as u32 && battery_charge[2] == 0 {
                        let _date = buffer.read_u32();
                        let value = buffer.read_u32();
                        battery_charge[2] = value as u8;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    } else {
                        let _date = buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    }
                }
                Ok(battery_charge)
            }
            Err(error) => Err(InverterError { message: error.message })
        }
    }

    pub fn get_battery_info(&mut self, socket: &Socket) -> Result<BatteryInfo, InverterError>
    {
        match self.get_data(socket, &Inverter::BATTERY_INFO) {
            Ok(mut buffer) => {
                let mut battery_info = BatteryInfo { temperature: [0,0,0], voltage: [0,0,0], current: [0,0,0] };

                while buffer.len() > buffer.get_rpos() {
                    let code = buffer.read_u32();
                    if code == 0 {
                        return Ok(battery_info);
                    }
                    let lri = code & 0x00FFFF00;
                    let _data_type = code >> 24;

                    if lri == BatTmpVal as u32 && battery_info.temperature[0] == 0 {
                        let _date = buffer.read_u32();
                        let value = buffer.read_u32();
                        battery_info.temperature[0] = value as u16;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    }
                    else if lri == BatTmpVal as u32 && battery_info.temperature[1] == 0 {
                        let _date = buffer.read_u32();
                        let value = buffer.read_u32();
                        battery_info.temperature[1] = value as u16;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    }
                    else if lri == BatTmpVal as u32 && battery_info.temperature[2] == 0 {
                        let _date = buffer.read_u32();
                        let value = buffer.read_u32();
                        battery_info.temperature[2] = value as u16;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    }
                    else if lri == BatAmp as u32 && battery_info.current[0] == 0 {
                        let _date = buffer.read_u32();
                        let value = buffer.read_i32();
                        battery_info.current[0] = value as i16;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    }
                    else if lri == BatAmp as u32 && battery_info.current[1] == 0 {
                        let _date = buffer.read_u32();
                        let value = buffer.read_i32();
                        battery_info.current[1] = value as i16;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    }
                    else if lri == BatAmp as u32 && battery_info.current[2] == 0 {
                        let _date = buffer.read_u32();
                        let value = buffer.read_i32();
                        battery_info.current[2] = value as i16;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    }
                    else if lri == BatVol as u32 && battery_info.voltage[0] == 0 {
                        let _date = buffer.read_u32();
                        let mut value = buffer.read_u32();
                        if value == 65535 {
                            value = 0;
                        }
                        battery_info.voltage[0] = value as u16;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    }
                    else if lri == BatVol as u32 && battery_info.voltage[1] == 0 {
                        let _date = buffer.read_u32();
                        let mut value = buffer.read_u32();
                        if value == 0xffffffff {
                            value = 0;
                        }
                        battery_info.voltage[1] = value as u16;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    }
                    else if lri == BatVol as u32 && battery_info.voltage[2] == 0 {
                        let _date = buffer.read_u32();
                        let mut value = buffer.read_u32();
                        println!("Value: {:x}", value);
                        if value == 65535 {
                            value = 0;
                        }
                        battery_info.voltage[2] = value as u16;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    }
                    else {
                        let _date = buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    }
                }
                Ok(battery_info)
            }
            Err(error) => Err(InverterError { message: error.message })
        }
    }

    pub fn get_dc_voltage(&mut self, socket: &Socket) -> Result<DCInfo, InverterError>
    {
        match self.get_data(socket, &Inverter::SPOT_DC_VOLTAGE) {
            Ok(mut buffer) => {

                let mut dc_info = DCInfo{voltage: [0,0], current: [0,0]};

                while buffer.len() > buffer.get_rpos() {
                    let code = buffer.read_u32();
                    if code == 0 {
                        return Ok(dc_info);
                    }
                    let lri = code & 0x00FFFF00;
                    let _data_type = code >> 24;

                    if lri == DcMsVol as u32 && dc_info.voltage[0] == 0 {
                        let _date = buffer.read_u32();
                        let value = buffer.read_u32();
                        dc_info.voltage[0] = value as u16;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    }
                    else if lri == DcMsVol as u32 && dc_info.voltage[1] == 0 {
                        let _date = buffer.read_u32();
                        let value = buffer.read_u32();
                        dc_info.voltage[1] = value as u16;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    } else if lri == DcMsAmp as u32 && dc_info.current[0] == 0 {
                        let _date = buffer.read_u32();
                        let value = buffer.read_u32();
                        dc_info.current[0] = value as u16;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    } else if lri == DcMsAmp as u32 && dc_info.current[1] == 0 {
                        let _date = buffer.read_u32();
                        let value = buffer.read_u32();
                        dc_info.current[1] = value as u16;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    } else {
                        println!("unhandled (dc voltage): {:x}", lri);
                        break;
                    }
                }
                Ok(dc_info)
            }
            Err(error) => Err(InverterError { message: error.message })
        }
    }

    pub fn get_energy_production(&mut self, socket: &Socket) -> Result<EnergyProductionInfo, InverterError>
    {
        match self.get_data(socket, &Inverter::ENERGY_PRODUCTION) {
            Ok(mut buffer) => {

                let mut ep_info = EnergyProductionInfo{daily_wh: 0, total_wh: 0};

                while buffer.len() > buffer.get_rpos() {
                    let code = buffer.read_u32();
                    if code == 0 {
                        return Ok(ep_info);
                    }
                    let lri = code & 0x00FFFF00;
                    let _data_type = code >> 24;
                    if lri == MeteringTotWhOut as u32 && ep_info.total_wh == 0 {
                        let _date = buffer.read_u32();
                        let value = buffer.read_u32();
                        ep_info.total_wh = value;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    }
                    else if lri == MeteringDyWhOut as u32 && ep_info.daily_wh == 0 {
                        let _date = buffer.read_u32();
                        let value = buffer.read_u32();
                        ep_info.daily_wh = value;
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                        buffer.read_u32();
                    } else {
                        println!("unhandled (energy production): {:x}", lri);
                        break;
                    }
                }
                Ok(ep_info)
            }
            Err(error) => {
                println!("Unsupported");
                Err(InverterError { message: error.message })
            }
        }
    }
}
