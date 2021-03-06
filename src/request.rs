use sys::ffi;
use std::{mem, slice};
use libc::{c_char, c_int, size_t};
use common::slice_to_mut_pair;

use {Method, Header, Headers, Version, Path, Chunks, ChunkReader};

/// A parsed Request, borrowing a RequestParser.
#[derive(Debug)]
pub struct Request<'s: 'h, 'h> {
    pub version: Version,
    pub method: Method<'s>,
    pub path: Path<'s>,
    pub headers: Headers<'s, 'h>,
    pub raw: &'s [u8]
}

#[derive(Debug)]
pub struct RequestParser<'s: 'h, 'h> {
    read: &'s [u8],
    unread: &'s mut [u8],
    headers: &'h mut [Header<'s>],
    method: Method<'s>,
    path: Path<'s>,
    version: c_int
}

#[derive(Debug, PartialEq, Copy)]
pub enum RequestParserError {
    ParseError,
    TooLong,
    IncompleteRequest
}

impl<'s, 'h> RequestParser<'s, 'h> {
    pub fn new(stream: &'s mut [u8], headers: &'h mut [Header<'s>]) -> RequestParser<'s, 'h> {
        let stream_start = stream.as_ptr();
        let read: &'s [u8] =
            unsafe { mem::transmute(slice::from_raw_parts(stream_start, 0)) };
        RequestParser {
            read: read,
            unread: stream,
            headers: headers,
            method: Method(&[]),
            path: Path(&[]),
            version: 0
        }
    }

    pub fn parse<C: Chunks, F>(mut self, chunks: C, cb: F)
    where F: FnOnce(Result<Request<'s, 'h>, RequestParserError>, C) {
        if self.unread.len() == 0 {
            return cb(Err(RequestParserError::TooLong), chunks);
        }

        chunks.chunk(move |reader| {
            let (mayberead, chunks) = reader.read(self.unread);
            let read = match mayberead {
                Some(read) => read,
                None => return cb(Err(RequestParserError::IncompleteRequest), chunks)
            };
            self.unread = &mut mem::replace(&mut self.unread, &mut [])[read..];
            unsafe { *slice_to_mut_pair(&mut self.read).1 += read; }

            let res = unsafe {
                let path_pair = slice_to_mut_pair(&mut self.path.0);
                let method_pair = slice_to_mut_pair(&mut self.method.0);

                ffi::phr_parse_request(
                    self.read.as_ptr() as *const c_char,
                    self.read.len() as size_t,
                    method_pair.0 as *mut *const u8 as *mut *const c_char,
                    method_pair.1 as *mut usize as *mut size_t,
                    path_pair.0 as *mut *const u8 as *mut *const c_char,
                    path_pair.1 as *mut usize as *mut size_t,
                    &mut self.version,
                    mem::transmute::<*mut Header,
                                     *mut ffi::phr_header>(self.headers.as_mut_ptr()),
                    slice_to_mut_pair(&mut &*self.headers).1 as *mut usize as *mut size_t,
                    (self.read.len() - read) as size_t
                )
            };

            match res {
                // Succesfully parsed, we're done.
                x if x > 0 => {
                    let req = Request {
                        version: Version(1, self.version as u8),
                        method: self.method,
                        path: self.path,
                        headers: Headers(self.headers),
                        raw: self.read
                    };

                    cb(Ok(req), chunks)
                },

                // Parse Error
                -1 => {
                    println!("Parse error on {:?}", self.read);
                    cb(Err(RequestParserError::ParseError), chunks)
                },

                // Incomplete, continue
                -2 => { self.parse(chunks, cb) },

                x => panic!("Unexpected result from phr_parse_request: {:?}", x)
            }
        })
    }
}
