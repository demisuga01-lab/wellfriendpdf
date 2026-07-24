using WellfriendPdf;

if (args.Length != 1)
{
    Console.Error.WriteLine("usage: Prompt02Reports <pdf>");
    return 2;
}

using var doc = WellfriendDocument.Open(args[0]);
Console.WriteLine(doc.SecurityReportJson());

var sanitized = doc.Sanitize("balanced");
var outPath = Path.ChangeExtension(args[0], ".sanitized.pdf");
File.WriteAllBytes(outPath, sanitized.Bytes);
Console.WriteLine(sanitized.ReportJson);
return 0;
