using WellfriendPdf;

if (args.Length != 1)
{
    Console.Error.WriteLine("usage: FormActionPolicyInteractiveDocx input.pdf");
    return 2;
}

using var document = WellfriendDocument.Open(File.ReadAllBytes(args[0]));
Console.WriteLine(document.FormJavaScriptReportJson());
Console.WriteLine(document.InteractiveDataReportJson());
Console.WriteLine(document.FormActionPolicyReportJson());
File.WriteAllBytes("form_action_policy-page-faithful.docx", document.ToDocx("page-faithful"));
return 0;
