use crate::client::UserClient;
use crate::cookies_io::CookiesIOExt;
use crate::fields::{DEFAULT_HEADERS, ROOT_SSO_URL, ROOT_VPN};
use crate::sso::sso_login;
use crate::types::{
    CbcAES128Enc, ElinkLoginInfo, ElinkServiceInfo, ElinkUserInfo, ElinkUserServiceInfo,
};
use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use base64::{prelude::BASE64_STANDARD, Engine};
use rand::Rng;
use reqwest::{cookie::Cookie, redirect::Policy, Client, StatusCode, Url};
use reqwest_cookie_store::CookieStoreMutex;
use std::{collections::HashMap, sync::Arc};
pub struct WebVpnClient {
    pub user: String,
    pub pwd: String,
    pub client: Arc<Client>,
    pub cookies: Arc<CookieStoreMutex>,
    login_info: Option<ElinkLoginInfo>,
    server_map: Option<HashMap<String, String>>,
}

impl WebVpnClient {
    pub fn new(user: String, pwd: String) -> Self {
        let cookies = Arc::new(CookieStoreMutex::default());
        WebVpnClient {
            user,
            pwd,
            client: Arc::new(
                Client::builder()
                    .cookie_provider(cookies.clone())
                    .redirect(Policy::none())
                    .build()
                    .unwrap(),
            ),
            cookies: cookies.clone(),
            login_info: None,
            server_map: None,
        }
    }

    /// SSO Login
    /// if Ok, return `ElinkUserInfo` else return Err(message)
    pub async fn sso_login(&mut self) -> Result<ElinkLoginInfo, String> {
        if let Ok(response) = sso_login(
            self.get_client(),
            self.get_cookies(),
            self.user.clone(),
            self.pwd.clone(),
            format!("{}/enlink/api/client/callback/cas", ROOT_VPN),
        )
        .await
        {
            if response.status() == StatusCode::FOUND {
                let redirect_location = response
                    .headers()
                    .get("Location")
                    .unwrap()
                    .to_str()
                    .unwrap();
                if let Ok(response) = self
                    .client
                    .get(redirect_location)
                    .headers(DEFAULT_HEADERS.clone())
                    .send()
                    .await
                {
                    if let Some(cookie) = &response
                        .cookies()
                        .filter(|cookie| cookie.name() == "clientInfo")
                        .collect::<Vec<Cookie>>()
                        .first()
                    {
                        let json =
                            String::from_utf8(BASE64_STANDARD.decode(cookie.value()).unwrap())
                                .unwrap();
                        //println!("{}", json);
                        let data: ElinkLoginInfo = serde_json::from_str(json.as_str()).unwrap();
                        self.login_info = Some(data.clone());
                        return Ok(data);
                    }
                }
            }
        }
        Err("()".into())
    }

    pub async fn common_login(&mut self) -> Result<ElinkLoginInfo, String> {
        let url = format!("{}/enlink/sso/login/submit", ROOT_VPN);
        const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let mut rng = rand::thread_rng();
        let mut token = (0..16)
            .map(|_| {
                let idx = rng.gen_range(0..CHARSET.len());
                CHARSET[idx] as u8
            })
            .collect::<Vec<u8>>();
        let iv = token.clone();
        token.reverse();
        let key = token.clone();
        let encryptor = CbcAES128Enc::new(key.as_slice().into(), iv.as_slice().into());
        let pwd_clone = self.pwd.clone();
        let raw_pwd = pwd_clone.as_bytes();
        let pwd_len = raw_pwd.len();
        let mut buf = [0u8; 256];
        buf[..pwd_len].copy_from_slice(&raw_pwd);
        let encrypt_buf = encryptor
            .encrypt_padded_mut::<Pkcs7>(&mut buf, pwd_len)
            .unwrap();
        let encrypt_pwd = BASE64_STANDARD.encode(encrypt_buf);
        let mut data: HashMap<&'static str, String> = HashMap::new();
        data.insert("username", self.user.clone());
        data.insert("password", encrypt_pwd);
        data.insert(
            "token",
            token.iter().map(|char| char.clone() as char).collect(),
        );
        data.insert("language", "zh-CN,zh;q=0.9,en;q=0.8".into());
        // Add Cookies here
        // self.cookies
        //    .add_cookie_str(cookie, &url.parse::<Url>().unwrap());
        if let Ok(response) = self
            .client
            .post(url)
            .header("Refer", format!("{}/enlink/sso/login", ROOT_VPN))
            .header("Origin", ROOT_VPN)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .headers(DEFAULT_HEADERS.clone())
            .form(&data)
            .send()
            .await
        {
            if response.status() == StatusCode::FOUND {
                if let Some(cookie) = &response
                    .cookies()
                    .filter(|cookie| cookie.name() == "clientInfo")
                    .collect::<Vec<Cookie>>()
                    .first()
                {
                    let json =
                        String::from_utf8(BASE64_STANDARD.decode(cookie.value()).unwrap()).unwrap();
                    let data: ElinkLoginInfo = serde_json::from_str(json.as_str()).unwrap();
                    self.login_info = Some(data.clone());
                    return Ok(data);
                }
            }
        };
        Err("普通登录失败，请检查账号密码是否错误...".into())
    }

    /// Please Call after login
    pub fn user_id(&self) -> String {
        self.login_info
            .clone()
            .expect("Please login first.")
            .userid
            .unwrap()
            .into()
    }

    /// Please Call after login
    pub async fn get_user_info(&self) -> Result<ElinkUserInfo, String> {
        if let Ok(response) = self
            .client
            .get(format!(
                "{}/enlink/api/client/user/findByUserId/{}",
                ROOT_VPN,
                self.user_id()
            ))
            .headers(DEFAULT_HEADERS.clone())
            .send()
            .await
        {
            if let Ok(json) = response.text().await {
                return Ok(serde_json::from_str(json.as_str()).unwrap());
            }
        }
        Err("获取失败，请稍后重试".into())
    }

    /// Please Call after login
    pub async fn get_tree_with_service(&mut self) -> Result<ElinkServiceInfo, String> {
        let mut body = HashMap::new();
        body.insert("nameLike", "".to_string());
        body.insert("serviceNameLike", "".to_string());
        body.insert("userId", self.user_id());
        if let Ok(response) = self
            .client
            .post(format!(
                "{}/enlink/api/client/service/group/treeWithService/",
                ROOT_VPN
            ))
            .headers(DEFAULT_HEADERS.clone())
            .header("Referer", format!("{}/enlink/", ROOT_VPN))
            .header("Origin", ROOT_VPN)
            .header("Content-Type", "application/json;charset=utf-8")
            .body(serde_json::to_string(&body).unwrap())
            .send()
            .await
        {
            if let Ok(json) = response.text().await {
                let services: ElinkServiceInfo = serde_json::from_str(json.as_str()).unwrap();
                let mut server_map: HashMap<String, String> = HashMap::new();
                services
                    .data
                    .clone()
                    .unwrap()
                    .service_all_list
                    .clone()
                    .into_iter()
                    .for_each(|element| {
                        element.into_iter().for_each(|element| {
                            server_map.insert(element.server.unwrap(), element.url_plus.unwrap());
                        })
                    });
                self.server_map = Some(server_map);
                return Ok(services);
            }
        }
        Err("获取失败，请稍后重试".into())
    }

    pub async fn get_service_by_user(&mut self) -> Result<ElinkUserServiceInfo, String> {
        let mut param = HashMap::new();
        param.insert("name", "");
        if let Ok(response) = self
            .client
            .get(format!(
                "{}/enlink/api/client/service/sucmp/findServiceByUserId/{}",
                ROOT_VPN,
                self.user_id()
            ))
            .headers(DEFAULT_HEADERS.clone())
            .header("Referer", format!("{}/enlink/", ROOT_VPN))
            .header("Origin", ROOT_VPN)
            .query(&param)
            .send()
            .await
        {
            if let Ok(json) = response.text().await {
                let services: ElinkUserServiceInfo = serde_json::from_str(json.as_str()).unwrap();
                let mut server_map: HashMap<String, String> = HashMap::new();

                services
                    .data
                    .clone()
                    .unwrap()
                    .into_iter()
                    .for_each(|element| {
                        server_map.insert(element.server.unwrap(), element.url_plus.unwrap());
                    });
                self.server_map = Some(server_map);
                return Ok(services);
            }
        }
        Err("获取失败，请稍后重试".into())
    }

    pub async fn get_visit_service_by_user(&mut self) -> Result<ElinkUserServiceInfo, String> {
        let mut param = HashMap::new();
        param.insert("name", "");
        if let Ok(response) = self
            .client
            .get(format!(
                "{}/enlink/api/client/service/suvisitmp/findVisitServiceByUserId/{}",
                ROOT_VPN,
                self.user_id()
            ))
            .headers(DEFAULT_HEADERS.clone())
            .header("Referer", format!("{}/enlink/", ROOT_VPN))
            .query(&param)
            .send()
            .await
        {
            if let Ok(json) = response.text().await {
                let services: ElinkUserServiceInfo = serde_json::from_str(json.as_str()).unwrap();
                let mut server_map: HashMap<String, String> = HashMap::new();

                services
                    .data
                    .clone()
                    .unwrap()
                    .into_iter()
                    .for_each(|element| {
                        server_map.insert(element.server.unwrap(), element.url_plus.unwrap());
                    });
                self.server_map = Some(server_map);
                return Ok(services);
            }
        }
        Err("获取失败，请稍后重试".into())
    }
}

impl UserClient for WebVpnClient {
    fn get_cookies(&self) -> Arc<CookieStoreMutex> {
        self.cookies.clone()
    }

    fn get_cookies_mut(&mut self) -> Arc<CookieStoreMutex> {
        self.cookies.clone()
    }

    fn redirect(&self, url: &str) -> String {
        if let Some(url_plus) = self.server_map.clone().unwrap().get(url) {
            return url_plus.clone();
        }
        url.to_string()
    }

    fn get_client(&self) -> Arc<Client> {
        self.client.clone()
    }

    fn get_client_mut(&mut self) -> Arc<Client> {
        self.client.clone()
    }

    fn initialize_url(&self, url: &str) {
        self.get_cookies()
            .lock()
            .unwrap()
            .copy_cookies(&ROOT_SSO_URL, &Url::parse(url).unwrap());
    }
}
