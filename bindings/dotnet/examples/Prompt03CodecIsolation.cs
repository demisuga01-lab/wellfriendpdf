using System;
using WellfriendPdf;

public static class Prompt03CodecIsolation
{
    public static int Main()
    {
        byte[] encoded = Convert.FromHexString("789ccb48cdc9c957c8afc84c49050019dd044e");
        string json = WellfriendDocument.CodecIsolationReportJson(
            "FlateDecode",
            encoded,
            "in_process");

        Console.WriteLine(json);
        return json.Contains("\"status\":\"success\"", StringComparison.Ordinal) ||
               json.Contains("\"status\": \"success\"", StringComparison.Ordinal)
            ? 0
            : 1;
    }
}
