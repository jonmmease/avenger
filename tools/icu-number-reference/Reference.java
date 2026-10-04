import com.ibm.icu.number.NumberFormatter;
import com.ibm.icu.util.ULocale;
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.util.Base64;

/** Emit one line per tab-separated locale, skeleton, and exact double bits: a label or an exception. */
class Reference {
    static String encode(String s) {
        return Base64.getEncoder().encodeToString(s.getBytes(StandardCharsets.UTF_8));
    }

    public static void main(String[] args) throws Exception {
        var lines = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        var out = new StringBuilder();
        for (String line; (line = lines.readLine()) != null;) {
            var fields = line.split("\t", -1);
            double value = Double.longBitsToDouble(Long.parseUnsignedLong(fields[2], 16));
            try {
                String result = NumberFormatter.forSkeleton(fields[1])
                    .locale(ULocale.forLanguageTag(fields[0])).format(value).toString();
                out.append("OK\t").append(encode(result)).append('\n');
            } catch (RuntimeException e) {
                out.append("ERR\t").append(encode(e.getClass().getSimpleName())).append('\n');
            }
        }
        System.out.print(out);
    }
}
