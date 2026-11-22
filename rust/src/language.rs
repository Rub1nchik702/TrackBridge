use std::borrow::Cow;

/// Older profiles store both translations in the note. Select the language at
/// display/delivery time so a language change also applies to queued refunds.
pub fn message<'a>(lang:&str,text:&'a str)->Cow<'a,str>{
    if let Some(service)=text.strip_prefix("Подключите аккаунт / Connect "){
        return format!("{} {service}",if lang=="en"{"Connect"}else{"Подключите аккаунт"}).into()
    }
    if let Some(detail)=text.strip_prefix("Ошибка сохранения профиля / Cannot save profile: "){
        return format!("{}: {detail}",if lang=="en"{"Cannot save profile"}else{"Ошибка сохранения профиля"}).into()
    }
    if let Some((ru,en))=text.split_once(" / "){
        if ru.chars().any(|c|('А'..='я').contains(&c)) && en.is_ascii(){
            return Cow::Borrowed(if lang=="en"{en}else{ru})
        }
    }
    if lang=="en"{return text.into()}

    Cow::Borrowed(match text{
        "Twitch login failed"=>"Не удалось подключить Twitch",
        "Twitch login expired"=>"Время подключения Twitch истекло",
        "Spotify login canceled"=>"Подключение Spotify отменено",
        "Spotify login expired"=>"Время подключения Spotify истекло",
        "Twitch Client ID mismatch"=>"Токен Twitch относится к другому Client ID",
        "Token missing"=>"Сервис не вернул токен доступа",
        "Cannot open browser"=>"Не удалось открыть браузер",
        "Connect original Twitch account"=>"Подключите Twitch-аккаунт, на котором был создан заказ",
        "Chat message not delivered"=>"Twitch не доставил сообщение в чат",
        "Invalid API data"|"Invalid API rows"=>"Сервис вернул некорректный список",
        "Invalid Spotify search response"=>"Spotify вернул некорректные результаты поиска",
        "Invalid viewer count"=>"Twitch не вернул число зрителей",
        "Missing duration"=>"Spotify не вернул длительность трека",
        "Missing explicit flag"=>"Spotify не вернул отметку Explicit",
        "Missing artists"=>"Spotify не вернул исполнителей",
        "Invalid metadata"=>"Spotify не вернул данные трека",
        "Invalid URI"=>"Некорректная ссылка Spotify",
        "Stop first"=>"Сначала остановите приём заказов",
        "Already running"=>"Приём уже запущен",
        "Invalid reward"=>"Введите название и стоимость награды",
        "Missing reward"|"Reward unavailable"=>"Награда Twitch недоступна",
        "Order unavailable"=>"Этот заказ недоступен для ручного решения",
        "Twitch subscription revoked"=>"Twitch отозвал подписку на события",
        "Twitch connection failed"|"Twitch disconnected"=>"Соединение с Twitch прервалось",
        "Missing welcome"=>"Twitch не подтвердил подключение",
        "Unexpected Twitch URL"=>"Twitch вернул неизвестный адрес подключения",
        "Repeated Twitch cursor"=>"Twitch повторно вернул ту же страницу покупок",
        "No confident track match found on Spotify. Points refunded."=>"Не удалось однозначно найти трек в Spotify. Баллы возвращены.",
        "You already have three active requests. Points refunded."=>"У зрителя уже три активных заказа. Баллы возвращены.",
        other=>other
    })
}
