plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "io.agents.pokeclaw.plugin"
    compileSdk = 36

    defaultConfig {
        minSdk = 28
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
        freeCompilerArgs += listOf("-Xskip-metadata-version-check")
    }

    buildFeatures {
        buildConfig = true
        compose = true
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.10.1")
    implementation("androidx.appcompat:appcompat:1.6.1")
    implementation(project(":tauri-android"))

    implementation("com.google.android.material:material:1.10.0")
    implementation("androidx.constraintlayout:constraintlayout:2.1.4")
    implementation("com.google.code.gson:gson:2.13.2")

    implementation("com.larksuite.oapi:oapi-sdk:2.5.3")
    implementation("com.dingtalk.open:app-stream-client:1.3.12")

    // LangChain4j
    implementation("dev.langchain4j:langchain4j-core:1.12.2")
    implementation("dev.langchain4j:langchain4j-open-ai:1.12.2") {
        exclude(group = "dev.langchain4j", module = "langchain4j-http-client-jdk")
    }
    implementation("dev.langchain4j:langchain4j-anthropic:1.12.2") {
        exclude(group = "dev.langchain4j", module = "langchain4j-http-client-jdk")
    }
    
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("com.squareup.okhttp3:logging-interceptor:4.12.0")
    implementation("com.squareup.retrofit2:retrofit:2.11.0")
    implementation("com.squareup.retrofit2:converter-gson:2.11.0")
    
    implementation("com.blankj:utilcodex:1.31.1")
    implementation("com.github.mrmike:ok2curl:0.8.0")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.6.2")
    implementation("androidx.lifecycle:lifecycle-viewmodel-ktx:2.6.2")
    implementation("com.tencent:mmkv-static:2.3.0")
    implementation("com.drakeet.multitype:multitype:4.3.0")
    implementation("com.github.bumptech.glide:glide:5.0.5")
    implementation("jp.wasabeef:glide-transformations:4.3.0")
    implementation("com.github.princekin-f:EasyFloat:2.0.4")
    
    // Jetpack Compose
    implementation(platform("androidx.compose:compose-bom:2025.05.00"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    debugImplementation("androidx.compose.ui:ui-tooling")

    // LiteRT-LM on-device LLM inference (Google AI Edge)
    implementation("com.google.ai.edge.litertlm:litertlm-android:0.10.0")

    // ZXing 二维码/条形码扫描
    implementation("com.google.zxing:core:3.5.3")

    // NanoHTTPD 嵌入式 HTTP 服务器（局域网配置服务）
    implementation("org.nanohttpd:nanohttpd:2.3.1")
}
