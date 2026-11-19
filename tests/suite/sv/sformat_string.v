// $sformat and $swrite into a string variable (IEEE 1800-2017 21.3.3: the output
// variable may be a string), with the format in a string variable, and into an element of
// a dynamic array of strings.
module m;
  string res, fmt;
  string arr [];
  int x = 5;
  bit signed [7:0] s8 [];
  initial begin
    fmt = "%0d";
    $sformat(res, fmt, x);
    if (res != "5") $display("FAILED -- sformat '%s'", res);
    $sformat(res, "<%0d,%s>", x + 1, "ab");
    if (res != "<6,ab>") $display("FAILED -- literal format '%s'", res);
    $swrite(res, x);
    if (res != "          5") $display("FAILED -- swrite '%s'", res);
    s8 = new[1];
    s8[0] = -1;
    $sformat(res, fmt, s8[0]);
    if (res != "-1") $display("FAILED -- signed element '%s'", res);
    arr = new[2];
    $sformat(arr[1], "x%0d", 7);
    if (arr[1] != "x7" || res.len() != 2) $display("FAILED -- element '%s'", arr[1]);
    $display("PASSED");
  end
endmodule
