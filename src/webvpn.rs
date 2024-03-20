use crate::fields::{DEFAULT_HEADERS, ROOT_SSO, ROOT_VPN};
use base64::{prelude::BASE64_STANDARD, Engine};
use reqwest::{
    cookie::{Cookie, Jar},
    redirect::Policy,
    Client, Url,
};
use scraper::{Html, Selector};
use std::{collections::HashMap, sync::Arc};

pub struct WebVpnClient {
    pub user: String,
    pub pwd: String,
    pub client: Client,
    pub cookies: Arc<Jar>,
}

fn parse_hidden_values(html: String) -> HashMap<String, String> {
    let mut hidden_values = HashMap::new();
    let dom = Html::parse_document(&*html);
    let input_hidden_selector = Selector::parse(r#"input[type="hidden"]"#).unwrap();
    let tags_hidden = dom.select(&input_hidden_selector);

    tags_hidden.for_each(|tag_hidden| {
        hidden_values.insert(
            tag_hidden.attr("name").unwrap().to_string(),
            tag_hidden.attr("value").unwrap().to_string(),
        );
    });

    hidden_values
}

impl WebVpnClient {
    pub fn new(user: &str, pwd: &str) -> Self {
        let cookies = Arc::new(Jar::default());
        WebVpnClient {
            user: user.to_string(),
            pwd: pwd.to_string(),
            client: Client::builder()
                .cookie_provider(cookies.clone())
                .redirect(Policy::none())
                .build()
                .unwrap(),
            cookies: cookies.clone(),
        }
    }

    /**
     * SSO 登录
     */
    pub async fn sso_login(&self) -> Result<String, String> {
        let mut j_session_id = String::new();
        let mut dom = String::new();

        let url = format!(
            "{}/sso/login?service={}/enlink/api/client/callback/cas",
            ROOT_SSO, ROOT_VPN
        );

        if let Ok(response) = self
            .client
            .get(url.clone())
            .headers(DEFAULT_HEADERS.clone())
            .send()
            .await
        {
            if let Some(cookie) = &response
                .cookies()
                .filter(|cookie| cookie.name() == "JSESSIONID")
                .collect::<Vec<Cookie>>()
                .first()
            {
                j_session_id = cookie.value().into();
            }

            if let Ok(text) = &response.text().await {
                dom = text.into();
            }
        }

        if j_session_id.is_empty() || dom.is_empty() {
            println!("j_session_id: {};dom: {}", j_session_id, dom);
            return Err("Sso登录失败(无法访问)，请尝试普通登录...".into());
        }

        let mut login_param = parse_hidden_values(dom);
        login_param.insert("username".into(), self.user.clone());
        login_param.insert("password".into(), BASE64_STANDARD.encode(self.pwd.clone()));
        self.cookies.add_cookie_str(
            &format!(
                "JSESSIONID={}; enter_login_url={}",
                j_session_id,
                urlencoding::encode(&url.clone())
            ),
            &ROOT_SSO.parse::<Url>().unwrap(),
        );
        if let Ok(response) = self
            .client
            .post(format!(
                "{}/sso/login;jsessionid={}",
                ROOT_SSO, j_session_id
            ))
            .headers(DEFAULT_HEADERS.clone())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&login_param)
            .send()
            .await
        {
            let u = response
                .headers()
                .get("Location")
                .unwrap()
                .to_str()
                .unwrap();
            println!("{}", u);
            if response.status() == 302 {
                if let Ok(response) = self
                    .client
                    .get(u)
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
                        return Ok(json);
                    }
                } else {
                    println!("登录失败")
                }
            }
        }

        Err("SSO 登录失败，请尝试普通登录...".into())
    }
}
