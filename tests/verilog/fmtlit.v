// String literals formatted as numbers; zero-padded widths
module top;
  reg [7:0] r;
  initial begin
    r = 65;
    $display("*|%d|%d|%b|%h|%s|", "A", "AB", "A", "ABCD", "AB");
    $display("*|", "text", "|");
    $display("*|%08d|%8d|%03d|%0d|%-6d|", r, r, r, r, r);
    $display("*|%c%c|", "x", 66);
  end
endmodule
