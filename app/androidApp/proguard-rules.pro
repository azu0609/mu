# JNA's native dispatcher reaches back into its own classes by name, and reads
# the UniFFI bindings' structures and callbacks by reflection.
-keep class com.sun.jna.** { *; }
-keep class * extends com.sun.jna.** { *; }
-dontwarn java.awt.**
