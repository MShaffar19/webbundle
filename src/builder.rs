// Copyright 2020 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::bundle::{Bundle, Exchange, Request, Response, Uri, Version};
use crate::prelude::*;
use headers::{ContentLength, ContentType, HeaderMapExt as _};
use http::StatusCode;
use std::path::{Path, PathBuf};
use url::Url;
use walkdir::WalkDir;

#[derive(Default)]
pub struct Builder {
    version: Option<Version>,
    primary_url: Option<Uri>,
    manifest: Option<Uri>,
    exchanges: Vec<Exchange>,
}

impl Builder {
    pub(crate) fn new() -> Self {
        Default::default()
    }

    /// Sets the version.
    pub fn version(mut self, version: Version) -> Self {
        self.version = Some(version);
        self
    }

    /// Sets the primary url.
    pub fn primary_url(mut self, primary_url: Uri) -> Self {
        self.primary_url = Some(primary_url);
        self
    }

    /// Sets the manifest url.
    pub fn manifest(mut self, manifest: Uri) -> Self {
        self.manifest = Some(manifest);
        self
    }

    /// Adds the exchange.
    pub fn exchange(mut self, exchange: Exchange) -> Self {
        self.exchanges.push(exchange);
        self
    }

    /// Append exchanges from the files under the given directory.
    ///
    /// `base_url` will be used as a prefix for each
    /// resource. A relative path from the given directory to each
    /// file is appended to `base_url`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use webbundle::{Bundle, Version};
    /// let bundle = Bundle::builder()
    ///        .version(Version::VersionB1)
    ///        .primary_url("https://example.com/index.html".parse()?)
    ///        .exchanges_from_dir("assets", "https://example.com".parse()?)?
    ///        .build()?;
    /// # Result::Ok::<(), anyhow::Error>(())
    /// ```
    pub fn exchanges_from_dir(mut self, dir: impl AsRef<Path>, base_url: Url) -> Result<Self> {
        self.exchanges.append(
            &mut ExchangeBuilder::new(PathBuf::from(dir.as_ref()), base_url)
                .walk()?
                .build(),
        );
        Ok(self)
    }

    /// Builds the bundle.
    pub fn build(self) -> Result<Bundle> {
        Ok(Bundle {
            version: self.version.context("no version")?,
            primary_url: self.primary_url.context("no primary_url")?,
            manifest: self.manifest,
            exchanges: self.exchanges,
        })
    }
}

#[allow(dead_code)]
struct ExchangeBuilder {
    base_url: Url,
    base_dir: PathBuf,
    exchanges: Vec<Exchange>,
}

#[allow(dead_code)]
impl ExchangeBuilder {
    fn new(base_dir: PathBuf, base_url: Url) -> Self {
        ExchangeBuilder {
            base_dir,
            base_url,
            exchanges: Vec::new(),
        }
    }

    fn walk(mut self) -> Result<Self> {
        for entry in WalkDir::new(&self.base_dir) {
            let entry = entry?;
            let file_type = entry.file_type();
            if file_type.is_symlink() {
                log::warn!(
                    "path is symbolink link. Skipping. {}",
                    entry.path().display()
                );
                continue;
            }
            if file_type.is_file() {
                let relative_path = pathdiff::diff_paths(entry.path(), &self.base_dir).unwrap();
                self = self.exchange(relative_path)?;
            }
        }
        Ok(self)
    }

    fn build(self) -> Vec<Exchange> {
        self.exchanges
    }

    fn url_from_relative_path(&self, relative_path: &Path) -> Result<Uri> {
        ensure!(
            relative_path.is_relative(),
            format!("Path is not relative: {}", relative_path.display())
        );
        Ok(self
            .base_url
            .join(&relative_path.display().to_string())?
            .to_string()
            .parse()?)
    }

    fn exchange(mut self, relative_path: impl AsRef<Path>) -> Result<Self> {
        self.exchanges.push(Exchange {
            request: Request::get(self.url_from_relative_path(relative_path.as_ref())?).body(())?,
            response: self.create_response(relative_path)?,
        });
        Ok(self)
    }

    fn create_response(&self, relative_path: impl AsRef<Path>) -> Result<Response> {
        ensure!(
            relative_path.as_ref().is_relative(),
            format!("Path is not relative: {}", relative_path.as_ref().display())
        );
        let path = self.base_dir.join(relative_path);
        let body = std::fs::read(&path)?;

        let content_length = ContentLength(body.len() as u64);
        let content_type = ContentType::from(mime_guess::from_path(&path).first_or_octet_stream());

        let mut response = Response::new(body);
        *response.status_mut() = StatusCode::OK;
        response.headers_mut().typed_insert(content_length);
        response.headers_mut().typed_insert(content_type);
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn build_fail() {
        assert!(Builder::new().build().is_err());
    }

    #[test]
    fn build() -> Result<()> {
        let bundle = Builder::new()
            .version(Version::Version1)
            .primary_url("https://example.com".parse()?)
            .build()?;
        assert_eq!(bundle.version, Version::Version1);
        assert_eq!(bundle.primary_url, "https://example.com".parse::<Uri>()?);
        Ok(())
    }

    #[test]
    fn exchange_builder() -> Result<()> {
        let base_dir = {
            let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            path.push("tests/builder");
            path
        };

        let exchanges = ExchangeBuilder::new(base_dir.clone(), "https://example.com/".parse()?)
            .exchange("index.html")?
            .build();
        assert_eq!(exchanges.len(), 1);
        let exchange = &exchanges[0];
        assert_eq!(
            exchange.request.uri(),
            &"https://example.com/index.html".parse::<Uri>()?
        );
        assert_eq!(exchange.response.status(), StatusCode::OK);
        assert_eq!(exchange.response.headers()["content-type"], "text/html");
        assert_eq!(
            exchange.response.headers()["content-length"],
            std::fs::read(base_dir.join("index.html"))?
                .len()
                .to_string()
        );
        assert_eq!(
            exchange.response.body(),
            &std::fs::read(base_dir.join("index.html"))?
        );
        Ok(())
    }

    #[test]
    fn walk() -> Result<()> {
        let base_dir = {
            let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            path.push("tests/builder");
            path
        };

        let exchanges = ExchangeBuilder::new(base_dir, "https://example.com/".parse()?)
            .walk()?
            .build();
        assert_eq!(exchanges.len(), 2);
        let urls = exchanges
            .into_iter()
            .map(|e| e.request.uri().to_string())
            .collect::<HashSet<_>>();
        assert!(urls.contains("https://example.com/index.html"));
        assert!(urls.contains("https://example.com/js/hello.js"));
        Ok(())
    }
}
