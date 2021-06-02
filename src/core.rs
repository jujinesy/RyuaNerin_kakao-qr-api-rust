use std::{collections::HashMap, net::AddrParseError};
use std::ffi::OsStr;
use std::fs::File;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, Duration};

use async_std::sync::Mutex;
use failure::Fallible;
use headless_chrome::{Browser, LaunchOptionsBuilder};
use headless_chrome::protocol::network::events::ResponseReceivedEventParams;
use hyper::{Body, HeaderMap, Method, Request, Response, StatusCode};
use lazy_static::lazy_static;
use qrcode_generator::QrCodeEcc;
use regex::bytes::Regex;
use reqwest::{Client, cookie::Jar};
use serde_derive::Deserialize;

use crate::err::HandlerError;

const USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 14_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 KAKAOTALK 9.0.3";

const DEFAULT_PNG_SIZE: u16 = 256; // Pixel
const TOKEN_EXPIRES: f32 = 14_f32; // Seconds

lazy_static! {
    static ref REG_TOKEN: Regex = Regex::new("\"token\":\\s*\"(.+?)\"").unwrap();
}

#[derive(Debug, Deserialize)]
struct Config {
    kakao_id: String,
    kakao_pw: String,
    api_key:  String,
    bind:     String,
}

#[derive(Debug)]
struct Cache {
    expires: SystemTime,
    token: String,
    map: HashMap<u16, Arc<Vec<u8>>>,
}

pub struct Handler {
    cfg: Config,
    client: Client,
    cookiejar: Arc<Jar>,
    browser: Mutex<Browser>,
    cache: Mutex<Cache>,
}

impl Handler {
    pub async fn new() -> Fallible<Self> {
        let cfg: Config = serde_json::from_reader(File::open("config.json")?)?;

        ////////////////////////////////////////////////////////////////////////////////////////////////////

        let browser_opt_extensions_useragent = format!(" user-agent=\"{}\"", USER_AGENT);

        let mut browser_opt_extensions: Vec<&OsStr> = Vec::new();
        browser_opt_extensions.push(OsStr::new(browser_opt_extensions_useragent.as_str()));

        let browser_opt = 
            LaunchOptionsBuilder::default()
            .sandbox(true)
            .window_size(Some((375, 667)))
            .extensions(browser_opt_extensions)
            .build()
            .expect("Couldn't find appropriate Chrome binary.");

        let browser = Browser::new(browser_opt)?;

        ////////////////////////////////////////////////////////////////////////////////////////////////////

        let mut client_header = HeaderMap::new();
        client_header.insert(
            reqwest::header::USER_AGENT,
            reqwest::header::HeaderValue::from_static(USER_AGENT)
        );

        let cookiejar = Arc::new(Jar::default());

        let client =
            reqwest::ClientBuilder::new()
            .cookie_provider(cookiejar.clone())
            .default_headers(client_header)
            .build()?;

        Ok(
            Handler {
                cfg,
                client,
                cookiejar : cookiejar.clone(),
                browser: Mutex::new(browser),
                cache: Mutex::new(Cache {
                    expires: SystemTime::now(),
                    token: String::new(),
                    map: HashMap::new(),
                }),
            }
        )
    }

    pub fn bind_addr(&self) -> Result<SocketAddr, AddrParseError> {
        return self.cfg.bind.parse::<SocketAddr>();
    }

    pub async fn serve(&self, req: Request<Body>, addr: SocketAddr) -> hyper::Result<Response<Body>> {
        fn resp(status: StatusCode) -> hyper::Result<Response<Body>> {
            Ok(Response::builder().status(status).body(Body::default()).unwrap())
        }

        let headers = req.headers();
        let params_get: HashMap<String, String> =
            req
            .uri()
            .query()
            .map(|v| url::form_urlencoded::parse(v.as_bytes()) .into_owned().collect())
            .unwrap_or_else(HashMap::new);

        let remote_addr =
            headers
            .get("X-Real-IP")
            .and_then(
                |hv| {
                    hv
                    .to_str()
                    .map(|hv_str| String::from(hv_str))
                    .ok()
                }
            )
            .unwrap_or(addr.ip().to_string());

        println!("{} {} {}", remote_addr, req.method(), req.uri());
        if req.method() != Method::GET {
            return resp(StatusCode::NOT_FOUND)
        }

        ////////////////////////////////////////////////// Check Api Key

        {
            let x_api_key =
                headers
                .get("X-API-KEY")
                .and_then(|x| x.to_str().ok())
                .and_then(|x| Some(x.to_string()))
                .unwrap_or(String::from(""));

            if x_api_key != self.cfg.api_key {
                println!("API Key is incorrect. IP: {}", remote_addr);
                return resp(StatusCode::UNAUTHORIZED)
            };
        }

        //////////////////////////////////////////////////

        let png_mode = match params_get.get("type") {
            Some(x) => match x.as_str() {
                "png" => true,
                "txt" => false,
                _ => return resp(StatusCode::BAD_REQUEST)
            }
            _ => false,
        };
        let png_size: u16 = match png_mode {
            false => DEFAULT_PNG_SIZE,
            true => match params_get.get("size") {
                Some(x) => match x.parse::<u16>() {
                    Ok(x) => x,
                    _ => return resp(StatusCode::BAD_REQUEST),
                },
                _ => DEFAULT_PNG_SIZE,
            }
        };

        //////////////////////////////////////////////////

        let mut cache = self.cache.lock().await;

        let now = SystemTime::now();
        if cache.expires < now {
            cache.map.clear();

            if cache.token != String::default() {
                cache.token = self.generate_token(false).await.unwrap_or(String::default());
            }

            if cache.token == String::default() {
                cache.token = match self.generate_token(true).await {
                    Ok(x) => x,
                    Err(err) => {
                        println!("Error : {}", err);
                        return resp(StatusCode::INTERNAL_SERVER_ERROR);
                    }
                };
            }

            cache.expires = now + Duration::from_secs_f32(TOKEN_EXPIRES);

        }

        if !png_mode {
            return Ok(
                Response::builder()
                .status(StatusCode::OK)
                .header(hyper::header::CONTENT_TYPE, "text/plain")
                .header(hyper::header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")
                .body(Body::from(cache.token.clone()))
                .unwrap()
            )
        } else {
            if !cache.map.contains_key(&png_size) {
                let qrcode = Arc::new(
                    match qrcode_generator::to_png_to_vec(
                        cache.token.as_str(),
                        QrCodeEcc::Medium,
                        png_size as usize,
                    ) {
                        Ok(x) => x,
                        Err(err) => {
                            println!("Error : {}", err);
                            return resp(StatusCode::INTERNAL_SERVER_ERROR)
                        },
                    }
                );
    
                cache.map.insert(png_size, qrcode.clone());
            }

            let qrcode = cache.map.get(&png_size).unwrap().clone();
            let qrcode_vec = Vec::from(qrcode.as_slice());

            return Ok(
                Response::builder()
                .status(StatusCode::OK)
                .header(hyper::header::CONTENT_TYPE, "image/png")
                .header(hyper::header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")
                .body(Body::from(qrcode_vec))
                .unwrap()
            )
        }
    }

    async fn generate_token(&self, do_login: bool) -> std::result::Result<String, HandlerError> {
        if do_login {
            let browser = self.browser.lock().await;
            let cookiejar = self.cookiejar.clone();

            let tab = browser.new_tab()?;
            tab.set_default_timeout(std::time::Duration::from_secs(30));
            tab.enable_response_handling(
                Box::new(
                    move |resp_event: ResponseReceivedEventParams, _| {
                        let url = match url::Url::parse(resp_event.response.url.as_str()) {
                            Ok(x) => x,
                            Err(_) => return,
                        };

                        for (k, v ) in resp_event.response.headers.iter() {
                            if k.to_lowercase() == "set-cookie" {
                                cookiejar.add_cookie_str(v, &url);
                            }
                        }
                    }
                )
            )?;

            tab.navigate_to("https://accounts.kakao.com/login?continue=https%3A%2F%2Faccounts.kakao.com%2Fweblogin%2Faccount%2Finfo")?;
            tab.wait_for_element("#login-form")?;

            let js = format!(
                " \
                    document.getElementById('id_email_2').value = '{}'; \
                    document.getElementById('id_password_3').value = '{}'; \
                ",
                self.cfg.kakao_id,
                self.cfg.kakao_pw,
            );
            tab.evaluate(js.as_str(), true)?;
            tab.wait_for_element("form#login-form button.submit")?.click()?;

            tab.wait_until_navigated()?;
        }

        //////////////////////////////////////////////////////////////////////////////////////////

        let resp =
            self
            .client
            .get("https://accounts.kakao.com/qr_check_in")
            .send()
            .await?;

        if resp.status() != StatusCode::OK {
            return Err(HandlerError::BadStatusCode(resp.status().as_u16()))
        }
        let body = resp.bytes().await?;
        let token_match = REG_TOKEN.captures(body.as_ref()).ok_or(HandlerError::CannotFindToken)?;
        let token = token_match.get(1).and_then(|x| String::from_utf8(Vec::from(x.as_bytes())).ok()).ok_or(HandlerError::CannotFindToken)?;

        //////////////////////////////////////////////////////////////////////////////////////////

        let resp =
            self
            .client
            .get(format!("https://accounts.kakao.com/qr_check_in/request_qr_data.json?lang=ko&os=ios&webview_v=2&is_under_age=false&token={}", token))
            .send()
            .await?;

        #[derive(Debug, Deserialize)]
        struct QRData {
            qr_data: String,
        }
        let qr_data: QRData = resp.json().await?;

        Ok(qr_data.qr_data)
    }
}
