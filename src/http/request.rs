use std::{io, str, fmt};

use tokio_core::io::EasyBuf;
use httparse;

use super::{Slice, Headers, Version};

pub struct Request {
    method: Slice,
    path: Slice,
    version: Version,
    // TODO: use a small vec to avoid this unconditional allocation
    headers: Vec<(Slice, Slice)>,
    data: EasyBuf,
}

impl Request {
    pub fn method(&self) -> &str {
        str::from_utf8(self.slice(&self.method)).unwrap()
    }

    pub fn path(&self) -> &str {
        str::from_utf8(self.slice(&self.path)).unwrap()
    }

    pub fn version(&self) -> Version {
        self.version
    }

    pub fn append_data(&mut self, buf: &[u8]) {
        self.data.get_mut().extend_from_slice(buf);
    }

    pub fn content_length(&self) -> Option<usize> {
        self.headers().content_length()
    }

    pub fn transfer_encoding_chunked(&self) -> bool {
        self.headers().transfer_encoding_chunked()
    }

    fn headers(&self) -> Headers {
        Headers {
            headers: self.headers.iter(),
            data: &self.data,
        }
    }

    fn slice(&self, slice: &Slice) -> &[u8] {
        &self.data.as_slice()[slice.0..slice.1]
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "<HTTP Request {} {}>", self.method(), self.path())
    }
}

pub fn decode(buf: &mut EasyBuf) -> io::Result<Option<Request>> {
    // TODO: we should grow this headers array if parsing fails and asks
    //       for more headers
    let (method, path, version, headers, amt) = {
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut r = httparse::Request::new(&mut headers);
        let status = try!(r.parse(buf.as_slice()).map_err(|e| {
            let msg = format!("failed to parse http request: {:?}", e);
            io::Error::new(io::ErrorKind::Other, msg)
        }));

        let amt = match status {
            httparse::Status::Complete(amt) => amt,
            httparse::Status::Partial => return Ok(None),
        };

        let toslice = |a: &[u8]| {
            let start = a.as_ptr() as usize - buf.as_slice().as_ptr() as usize;
            assert!(start < buf.len());
            (start, start + a.len())
        };

        (toslice(r.method.unwrap().as_bytes()),
         toslice(r.path.unwrap().as_bytes()),
         r.version.unwrap(),
         r.headers
          .iter()
          .map(|h| (toslice(h.name.as_bytes()), toslice(h.value)))
          .collect(),
         amt)
    };

    let version = match version {
        0 => Version::Http10,
        1 => Version::Http11,
        _ => unimplemented!()
    };

    Ok(Request {
        method: method,
        path: path,
        version: version,
        headers: headers,
        data: buf.drain_to(amt),
    }.into())
}

pub fn encode(msg: Request, buf: &mut Vec<u8>) {
    buf.extend_from_slice(msg.data.as_slice());
}
