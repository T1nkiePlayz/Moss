//! Making sure an image is present locally, with progress shown in an operation.
use crate::{ops::OpCtx, state::St};
use anyhow::{Result, anyhow, bail};
use std::time::{Duration, Instant};

/// Pull `image` unless it is already there. `image` must come from `image::validate`/`image::for_*`.
pub async fn ensure(st: &St, ctx: &OpCtx, image: &str) -> Result<()> {
    if image::validate(image).as_deref() != Ok(image) { bail!("refusing to pull an image that is not on the allow-list"); }
    let eng = super::engine(st).await.map_err(|m| anyhow!(m))?;
    let (img, ctx2) = (image.to_string(), ctx.clone());
    tokio::task::spawn_blocking(move || -> Result<()> {
        if eng.image_present(&img)? { ctx2.log(format!("{img} is already downloaded")); return Ok(()); }
        ctx2.stage(&format!("Downloading {img}"));
        let mut last = Instant::now() - Duration::from_secs(1);
        eng.pull(&img, &mut |line| {
            if !line.is_empty() && last.elapsed() >= Duration::from_millis(400) { ctx2.log(line.to_string()); last = Instant::now(); }
            !ctx2.cancelled()
        }).map_err(|e| if ctx2.cancelled() { anyhow!("cancelled") } else { e })
    }).await?
}

use super::image;
