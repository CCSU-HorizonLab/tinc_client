package com.tinclink.android.auth

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class LoginInputValidatorTest {
    @Test
    fun normalizesTheDesktopAccessCodeContract() {
        assertEquals("TL-7K9M-2Q4F", LoginInputValidator.normalizeAccessCode(" tl-7k9m-2q4f "))
        assertEquals(null, LoginInputValidator.normalizeAccessCode("TL-7K90-2Q4F"))
        assertEquals(null, LoginInputValidator.normalizeAccessCode("TL-7K9M_2Q4F"))
    }

    @Test
    fun acceptsOnlyAServiceRootWithoutEmbeddedCredentials() {
        assertEquals(
            "https://vpn.example.com:8443/",
            LoginInputValidator.normalizeOrigin("HTTPS://VPN.EXAMPLE.COM:8443/", false),
        )
        assertEquals(null, LoginInputValidator.normalizeOrigin("https://vpn.example.com/api", false))
        assertEquals(null, LoginInputValidator.normalizeOrigin("https://user:pass@vpn.example.com", false))
        assertEquals(null, LoginInputValidator.normalizeOrigin("https://vpn.example.com/?token=x", false))
    }

    @Test
    fun cleartextRequiresAnExplicitChoice() {
        assertEquals(null, LoginInputValidator.normalizeOrigin("http://192.0.2.1:83", false))
        assertEquals(
            "http://192.0.2.1:83/",
            LoginInputValidator.normalizeOrigin("http://192.0.2.1:83", true),
        )
    }

    @Test
    fun passwordLimitUsesUtf8Bytes() {
        val valid = LoginInputValidator.validate(
            "https://vpn.example.com",
            "TL-7K9M-2Q4F",
            "密".repeat(341),
        )
        val tooLarge = LoginInputValidator.validate(
            "https://vpn.example.com",
            "TL-7K9M-2Q4F",
            "密".repeat(342),
        )

        assertTrue(valid is LoginInputValidation.Valid)
        assertTrue(tooLarge is LoginInputValidation.Invalid)
    }
}
