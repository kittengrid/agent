// GET /sys/hello
//
// Description: Returns the cutiest Http response
#[get("/sys/hello")]
pub fn hello() -> String {
    String::from("kitty")
}

#[cfg(test)]
mod test {
    use crate::rocket;
    use rocket::http::Status;
    use rocket::local::blocking::Client;

    #[test]
    fn hello() {
        let client = Client::tracked(rocket()).expect("valid rocket instance");
        let response = client.get(uri!(super::hello)).dispatch();
        assert_eq!(response.status(), Status::Ok);
        assert_eq!(response.into_string().unwrap(), "kitty");
    }
}
