package com.tinclink.android.auth

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LoginApiTest {
    private val valid = """{
        "status":1,"sid":"android_a","net_name":"office","node_ip":"10.0.11.213",
        "access_code":"TL-7K9M-2Q4F","token":"secret-token",
        "allowed_routes":["10.0.11.1/32"],"route_revision":"routes-1",
        "route_lease_seconds":120
    }"""

    @Test
    fun acceptsServerContractWithoutExposingToken() {
        val result = LoginApi.parseSuccess(valid, "TL-7K9M-2Q4F")
        assertEquals(LoginResult.Success("android_a", "office", "10.0.11.213"), result)
        assertFalse(result.toString().contains("secret-token"))
    }

    @Test
    fun rejectsMissingIdentityAndMismatchedAccessCode() {
        assertTrue(LoginApi.parseSuccess(valid.replace("\"token\":\"secret-token\",", ""), "TL-7K9M-2Q4F") is LoginResult.Failure)
        assertTrue(LoginApi.parseSuccess(valid, "TL-9A8B-7C6D") is LoginResult.Failure)
        assertTrue(LoginApi.parseSuccess(valid.replace("10.0.11.213", "999.0.11.213"), "TL-7K9M-2Q4F") is LoginResult.Failure)
    }

    @Test
    fun rejectedOrMalformedResponsesNeverReportSuccess() {
        assertTrue(LoginApi.parseSuccess(valid.replace("\"status\":1", "\"status\":0"), "TL-7K9M-2Q4F") is LoginResult.Failure)
        assertTrue(LoginApi.parseSuccess("{", "TL-7K9M-2Q4F") is LoginResult.Failure)
    }

    @Test
    fun requestUsesAccessCodeWithoutSid() {
        val input = ValidatedLoginInput("https://vpn.example.com/", "TL-7K9M-2Q4F", "test-password")
        val request = JSONObject(LoginApi.requestBody(input))
        assertEquals("TL-7K9M-2Q4F", request.getString("accessCode"))
        assertEquals("test-password", request.getString("password"))
        assertFalse(request.has("sid"))
    }
}
