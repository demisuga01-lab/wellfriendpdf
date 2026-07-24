using WellfriendPdf;

if (args.Length != 1)
{
    Console.Error.WriteLine("usage: Prompt19InteractiveDocx input.pdf");
    return 2;
}

using var document = WellfriendDocument.Open(File.ReadAllBytes(args[0]));
Console.WriteLine(document.FormJavaScriptReportJson());
Console.WriteLine(document.InteractiveDataReportJson());
Console.WriteLine(document.Prompt19ReportJson());
File.WriteAllBytes("prompt19-page-faithful.docx", document.ToDocx("page-faithful"));
return 0;
