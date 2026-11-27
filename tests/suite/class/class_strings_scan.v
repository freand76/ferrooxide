// Methods of string elements of a queue property (`q[i].len()`, `q[i].substr()`),
// streams of strings and queues of strings into a string (IEEE 1800-2017 11.4.14),
// $sscanf into a class property and %s / %c into string variables (21.3.4.3), and
// output arguments that are properties.
class C;
  protected string m_argv[$];
  int verbosity;
  string word;
  function new;
    m_argv.push_back("abc");
    m_argv.push_back("+x=1");
    m_argv.push_back("ab");
  endfunction
  function int count(string match);
    int len = match.len();
    int n = 0;
    foreach (m_argv[i])
      if ((m_argv[i].len() >= len) && (m_argv[i].substr(0, len - 1) == match)) n++;
    return n;
  endfunction
  function string joined();
    return {>>{m_argv}};
  endfunction
endclass

module t;
  integer errors = 0;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  task checks(input integer line, input string got, input string want);
    if (got != want) begin
      $display("FAILED line %0d: got '%s' want '%s'", line, got, want);
      errors = errors + 1;
    end
  endtask
  function automatic string join_ref(ref string i[$]);
    join_ref = {>>{i}};
  endfunction
  task automatic get2(output int a, output string b);
    a = 9;
    b = "nine";
  endtask

  string q[$];
  string s, src;
  int code, v;
  C c;

  initial begin
    c = new;
    check(`__LINE__, c.count("ab"), 2);
    check(`__LINE__, c.count("+"), 1);
    checks(`__LINE__, c.joined(), "abc+x=1ab");
    q = '{"ab", "", "cde"};
    checks(`__LINE__, join_ref(q), "abcde");
    s = {>>{"x", q, "y"}};
    checks(`__LINE__, s, "xabcdey");
    s = {>>{"u", s}};
    checks(`__LINE__, s, "uxabcdey");
    code = $sscanf("42 ab", "%d %s", v, s);
    check(`__LINE__, code, 2);
    check(`__LINE__, v, 42);
    checks(`__LINE__, s, "ab");
    src = "7 xyz q";
    code = $sscanf(src, "%d %s %c", c.verbosity, c.word, s);
    check(`__LINE__, code, 3);
    check(`__LINE__, c.verbosity, 7);
    checks(`__LINE__, c.word, "xyz");
    checks(`__LINE__, s, "q");
    get2(c.verbosity, c.word);
    check(`__LINE__, c.verbosity, 9);
    checks(`__LINE__, c.word, "nine");
    if (errors == 0) $display("PASSED");
  end
endmodule
