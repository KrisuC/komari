use std::time::Duration;

use anyhow::{Error, Ok, bail};
use bit_vec::BitVec;
use input::key_input_client::KeyInputClient;
pub use input::{Coordinate, Key, KeyState, MouseAction};
use input::{KeyDownRequest, KeyInitRequest, KeyRequest, KeyUpRequest, MouseRequest};
use tokio::runtime::Handle;
use tokio::task::block_in_place;
use tokio::time::timeout;
use tonic::Request;
use tonic::transport::{Channel, Endpoint};

use crate::rpc::input::KeyStateRequest;

mod input {
    tonic::include_proto!("input");
}

#[derive(Debug)]
pub struct InputService {
    client: KeyInputClient<Channel>,
    key_down: BitVec, // TODO: is a bit wrong good?
    mouse_coordinate: Coordinate,
}

impl InputService {
    pub fn connect<D>(dest: D) -> Result<Self, Error>
    where
        D: TryInto<Endpoint>,
        D: AsRef<str>,
        D::Error: std::error::Error + Send + Sync + 'static,
    {
        let endpoint = TryInto::<Endpoint>::try_into(dest.as_ref().to_string())?;
        let client = block_future(async move {
            timeout(Duration::from_secs(3), KeyInputClient::connect(endpoint)).await
        })??;
        Ok(Self {
            client,
            key_down: BitVec::from_elem(128, false),
            mouse_coordinate: Coordinate::Screen,
        })
    }

    fn reset(&mut self) {
        for i in 0..self.key_down.len() {
            if Key::try_from(i as i32).is_ok() {
                let _ = block_future(async {
                    self.client
                        .send_up(Request::new(KeyUpRequest { key: i as i32 }))
                        .await
                });
            }
        }
        self.key_down.clear();
    }

    pub fn init(&mut self, seed: &[u8]) -> Result<(), Error> {
        let response = block_future(async {
            self.client
                .init(KeyInitRequest {
                    seed: seed.to_vec(),
                })
                .await
        })?
        .into_inner();
        self.mouse_coordinate = response.mouse_coordinate();
        Ok(())
    }

    pub fn mouse_coordinate(&self) -> Coordinate {
        self.mouse_coordinate
    }

    pub fn key_state(&mut self, key: Key) -> Result<KeyState, Error> {
        block_future(async move {
            let request = Request::new(KeyStateRequest { key: key.into() });
            let response = self.client.key_state(request).await?.into_inner();

            Ok(KeyState::try_from(response.state)?)
        })
    }

    pub fn send_mouse(
        &mut self,
        width: i32,
        height: i32,
        x: i32,
        y: i32,
        action: MouseAction,
    ) -> Result<(), Error> {
        Ok(block_future(async move {
            self.client
                .send_mouse(Request::new(MouseRequest {
                    width,
                    height,
                    x,
                    y,
                    action: action.into(),
                }))
                .await?;
            Ok(())
        })?)
    }

    pub fn send_key(&mut self, key: Key, down_ms: f32) -> Result<(), Error> {
        Ok(block_future(async move {
            let request = Request::new(KeyRequest {
                key: key.into(),
                down_ms,
            });

            self.client.send(request).await?;
            self.key_down.set(i32::from(key) as usize, false);
            Ok(())
        })?)
    }

    pub fn send_key_up(&mut self, key: Key) -> Result<(), Error> {
        if !self.can_send_key(key, false) {
            bail!("key not sent");
        }
        Ok(block_future(async move {
            let request = Request::new(KeyUpRequest { key: key.into() });

            self.client.send_up(request).await?;
            self.key_down.set(i32::from(key) as usize, false);
            Ok(())
        })?)
    }

    pub fn send_key_down(&mut self, key: Key) -> Result<(), Error> {
        if !self.can_send_key(key, true) {
            bail!("key not sent");
        }
        Ok(block_future(async move {
            let request = Request::new(KeyDownRequest { key: key.into() });

            self.client.send_down(request).await?;
            self.key_down.set(i32::from(key) as usize, true);
            Ok(())
        })?)
    }

    #[inline]
    fn can_send_key(&self, key: Key, is_down: bool) -> bool {
        let key_num = i32::from(key) as usize;
        let was_down = self.key_down.get(key_num).unwrap();
        !matches!((was_down, is_down), (true, true) | (false, false))
    }
}

impl Drop for InputService {
    fn drop(&mut self) {
        self.reset();
    }
}

#[inline]
fn block_future<F: Future>(f: F) -> F::Output {
    block_in_place(|| Handle::current().block_on(f))
}

#[cfg(test)]
mod test {
    // TODO HOW TO?
}
