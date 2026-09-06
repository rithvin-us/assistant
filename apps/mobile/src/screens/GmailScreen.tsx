/**



 * Gmail Screen — Search & Read.



 *



 * Implements:



 * - Multi-account selection for strict account isolation



 * - Search with native Gmail query syntax (from:, to:, subject:, is:unread)



 * - Email summary feed (sender, subject, snippet, date, unread indicator)



 * - On-demand email reader drawer (privacy preserved, no DB mirror)



 * - Pure Light Theme aesthetics



 */







import { useState, useEffect, useCallback } from "react";



import Box from "@mui/material/Box";



import Typography from "@mui/material/Typography";



import TextField from "@mui/material/TextField";



import InputAdornment from "@mui/material/InputAdornment";





import Chip from "@mui/material/Chip";



import IconButton from "@mui/material/IconButton";



import Drawer from "@mui/material/Drawer";



import CircularProgress from "@mui/material/CircularProgress";



import Card from "@mui/material/Card";



import CardContent from "@mui/material/CardContent";



import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";



import RefreshRoundedIcon from "@mui/icons-material/RefreshRounded";



import SearchRoundedIcon from "@mui/icons-material/SearchRounded";



import MailOutlineRoundedIcon from "@mui/icons-material/MailOutlineRounded";



import CloseRoundedIcon from "@mui/icons-material/CloseRounded";







import type { AccountSummary, EmailDetail, EmailSummary } from "../api/types";



import { fetchGoogleAccounts, searchGmail, readGmail } from "../api/google";







interface GmailScreenProps {



  onBack?: () => void;



}







export default function GmailScreen({ onBack }: GmailScreenProps) {



  const [accounts, setAccounts] = useState<AccountSummary[]>([]);



  const [selectedAccountId, setSelectedAccountId] = useState<string>("");



  const [searchQuery, setSearchQuery] = useState("is:unread");



  const [emails, setEmails] = useState<EmailSummary[]>([]);



  const [loading, setLoading] = useState(false);







  // Selected Email for Detail View



  const [selectedEmailId, setSelectedEmailId] = useState<string | null>(null);



  const [emailDetail, setEmailDetail] = useState<EmailDetail | null>(null);



  const [loadingDetail, setLoadingDetail] = useState(false);







  const [errorMessage, setErrorMessage] = useState<string | null>(null);







  // Load Accounts



  useEffect(() => {



    async function init() {



      try {



        const accs = await fetchGoogleAccounts();



        setAccounts(accs);



        if (accs.length > 0 && !selectedAccountId) {



          setSelectedAccountId(accs[0].id);



        }



      } catch (err: unknown) {



        const msg = err instanceof Error ? err.message : "Failed to load accounts";



        setErrorMessage(msg);



      }



    }



    init();



  }, [selectedAccountId]);







  // Execute Search



  const handleSearch = useCallback(async () => {



    if (!selectedAccountId) {



      setEmails([]);



      return;



    }



    try {



      setLoading(true);



      setErrorMessage(null);



      const results = await searchGmail(selectedAccountId, searchQuery.trim() || "in:inbox", 25);



      setEmails(results);



    } catch (err: unknown) {



      const msg = err instanceof Error ? err.message : "Gmail search failed";



      setErrorMessage(msg);



    } finally {



      setLoading(false);



    }



  }, [selectedAccountId, searchQuery]);







  useEffect(() => {



    if (selectedAccountId) {



      // eslint-disable-next-line react-hooks/set-state-in-effect
      handleSearch();



    }



  }, [selectedAccountId, handleSearch]);







  // Open Email Detail



  const handleOpenEmail = async (email: EmailSummary) => {



    if (!selectedAccountId) return;



    setSelectedEmailId(email.id);



    setEmailDetail(null);



    try {



      setLoadingDetail(true);



      const detail = await readGmail(selectedAccountId, email.id);



      setEmailDetail(detail);



    } catch (err: unknown) {



      const msg = err instanceof Error ? err.message : "Failed to read email";



      setErrorMessage(msg);



    } finally {



      setLoadingDetail(false);



    }



  };







  const formatDateLabel = (dateStr?: string | null) => {



    if (!dateStr) return "";



    try {



      const d = new Date(dateStr);



      return d.toLocaleDateString([], { month: "short", day: "numeric" });



    } catch {



      return dateStr.slice(0, 10);



    }



  };







  return (



    <Box



      sx={{



        display: "flex",



        flexDirection: "column",



        height: "100%",



        bgcolor: "#FFFFFF",



      }}



    >



      {/* Header */}



      <Box



        sx={{



          display: "flex",



          alignItems: "center",



          justifyContent: "space-between",



          px: 2,



          py: 1.5,



          borderBottom: "1px solid #F0F0F0",



        }}



      >



        <Box sx={{ display: "flex", alignItems: "center", gap: 1.5 }}>



          {onBack && (



            <IconButton onClick={onBack} size="small" edge="start" sx={{ color: "#202020" }}>



              <ArrowBackRoundedIcon />



            </IconButton>



          )}



          <Box>



            <Typography variant="h6" sx={{ fontWeight: 700, color: "#202020", fontSize: "1.15rem", lineHeight: 1.2 }}>



              Gmail



            </Typography>



            <Typography variant="caption" sx={{ color: "#808080" }}>



              Search & read emails



            </Typography>



          </Box>



        </Box>



        <IconButton onClick={handleSearch} size="small" sx={{ color: "#606060" }}>



          <RefreshRoundedIcon />



        </IconButton>



      </Box>







      {/* Account Selector Bar */}



      {accounts.length > 0 && (



        <Box sx={{ px: 2, pt: 1.5, pb: 0.5, display: "flex", alignItems: "center", gap: 1, overflowX: "auto" }}>



          <Typography variant="caption" sx={{ color: "#888888", fontWeight: 600, flexShrink: 0 }}>



            Account:



          </Typography>



          {accounts.map((acc) => {



            const isSelected = acc.id === selectedAccountId;



            return (



              <Chip



                key={acc.id}



                label={acc.email}



                clickable



                onClick={() => setSelectedAccountId(acc.id)}



                size="small"



                sx={{



                  fontWeight: isSelected ? 700 : 500,



                  bgcolor: isSelected ? "#202020" : "#F4F4F4",



                  color: isSelected ? "#FFFFFF" : "#505050",



                  "&:hover": { bgcolor: isSelected ? "#333333" : "#EAEAEA" },



                }}



              />



            );



          })}



        </Box>



      )}







      {/* Search Input Bar */}



      <Box sx={{ px: 2, py: 1.5 }}>



        <TextField



          fullWidth



          size="small"



          placeholder="Search mail (e.g. is:unread, from:prof, subject:exam)"



          value={searchQuery}



          onChange={(e) => setSearchQuery(e.target.value)}



          onKeyDown={(e) => {



            if (e.key === "Enter") handleSearch();



          }}



          slotProps={{



            input: {



              startAdornment: (



                <InputAdornment position="start">



                  <SearchRoundedIcon fontSize="small" sx={{ color: "#808080" }} />



                </InputAdornment>



              ),



              sx: {



                borderRadius: 2.5,



                bgcolor: "#F7F7F7",



                fontSize: "0.85rem",



                "& fieldset": { borderColor: "#EBEBEB" },



              },



            },



          }}



        />







        {/* Quick Query Filters */}



        <Box sx={{ display: "flex", gap: 0.75, mt: 1, overflowX: "auto" }}>



          {[



            { label: "Unread", query: "is:unread" },



            { label: "Starred", query: "is:starred" },



            { label: "Important", query: "is:important" },



            { label: "Today", query: "newer_than:1d" },



          ].map((item) => (



            <Chip



              key={item.label}



              label={item.label}



              size="small"



              clickable



              onClick={() => {



                setSearchQuery(item.query);



              }}



              sx={{



                fontSize: "0.72rem",



                fontWeight: searchQuery === item.query ? 700 : 500,



                bgcolor: searchQuery === item.query ? "#DC4C3E" : "#F0F0F0",



                color: searchQuery === item.query ? "#FFFFFF" : "#404040",



              }}



            />



          ))}



        </Box>



      </Box>







      {/* Main Email List */}



      <Box sx={{ flex: 1, overflowY: "auto", px: 2, pb: 2 }}>



        {errorMessage && (



          <Box



            sx={{



              p: 1.5,



              mb: 2,



              borderRadius: 2,



              bgcolor: "#FFF4F2",



              border: "1px solid #FFEBE8",



              color: "#DC4C3E",



              fontSize: "0.85rem",



            }}



          >



            {errorMessage}



          </Box>



        )}







        {accounts.length === 0 ? (



          <Box sx={{ textAlign: "center", py: 6, color: "#808080" }}>



            <MailOutlineRoundedIcon sx={{ fontSize: 44, color: "#CCCCCC", mb: 1 }} />



            <Typography variant="body2">No connected Google accounts found.</Typography>



            <Typography variant="caption" sx={{ color: "#AAAAAA" }}>



              Connect a Google account to read Gmail.



            </Typography>



          </Box>



        ) : loading ? (



          <Box sx={{ display: "flex", justifyContent: "center", py: 5 }}>



            <CircularProgress size={24} sx={{ color: "#DC4C3E" }} />



          </Box>



        ) : emails.length === 0 ? (



          <Box sx={{ textAlign: "center", py: 6, color: "#808080" }}>



            <Typography variant="body2">No emails found matching query.</Typography>



          </Box>



        ) : (



          <Box sx={{ display: "flex", flexDirection: "column", gap: 1 }}>



            {emails.map((m) => (



              <Card



                key={m.id}



                variant="outlined"



                onClick={() => handleOpenEmail(m)}



                sx={{



                  borderRadius: 2,



                  borderColor: m.is_unread ? "#E0E0E0" : "#F0F0F0",



                  bgcolor: m.is_unread ? "#FFFFFF" : "#FAFAFA",



                  cursor: "pointer",



                  transition: "background 0.15s, border-color 0.15s",



                  "&:hover": { bgcolor: "#F5F5F5" },



                }}



              >



                <CardContent sx={{ p: 1.5, "&:last-child": { pb: 1.5 } }}>



                  <Box sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", mb: 0.5 }}>



                    <Box sx={{ display: "flex", alignItems: "center", gap: 0.75, flex: 1, minWidth: 0 }}>



                      {m.is_unread && (



                        <Box sx={{ width: 7, height: 7, borderRadius: "50%", bgcolor: "#1A73E8", flexShrink: 0 }} />



                      )}



                      <Typography



                        variant="caption"



                        sx={{



                          fontWeight: m.is_unread ? 700 : 500,



                          color: m.is_unread ? "#202020" : "#606060",



                          overflow: "hidden",



                          textOverflow: "ellipsis",



                          whiteSpace: "nowrap",



                        }}



                      >



                        {m.from}



                      </Typography>



                    </Box>



                    <Typography variant="caption" sx={{ color: "#999999", flexShrink: 0, fontSize: "0.72rem" }}>



                      {formatDateLabel(m.date)}



                    </Typography>



                  </Box>







                  <Typography



                    variant="subtitle2"



                    sx={{



                      fontWeight: m.is_unread ? 700 : 500,



                      color: "#202020",



                      fontSize: "0.85rem",



                      lineHeight: 1.25,



                      mb: 0.5,



                      overflow: "hidden",



                      textOverflow: "ellipsis",



                      whiteSpace: "nowrap",



                    }}



                  >



                    {m.subject || "(No Subject)"}



                  </Typography>







                  <Typography



                    variant="body2"



                    sx={{



                      color: "#777777",



                      fontSize: "0.78rem",



                      lineHeight: 1.3,



                      display: "-webkit-box",



                      WebkitLineClamp: 2,



                      WebkitBoxOrient: "vertical",



                      overflow: "hidden",



                    }}



                  >



                    {m.snippet}



                  </Typography>



                </CardContent>



              </Card>



            ))}



          </Box>



        )}



      </Box>







      {/* Email Reader Drawer */}



      <Drawer



        anchor="bottom"



        open={Boolean(selectedEmailId)}



        onClose={() => setSelectedEmailId(null)}



        slotProps={{



          paper: {



            sx: {



              height: "85vh",



              borderTopLeftRadius: 20,



              borderTopRightRadius: 20,



              bgcolor: "#FFFFFF",



              display: "flex",



              flexDirection: "column",



            },



          },



        }}



      >



        {/* Reader Header */}



        <Box



          sx={{



            display: "flex",



            alignItems: "center",



            justifyContent: "space-between",



            p: 2,



            borderBottom: "1px solid #F0F0F0",



          }}



        >



          <Typography variant="subtitle1" sx={{ fontWeight: 700, color: "#202020", flex: 1, pr: 1 }} noWrap>



            {emailDetail?.subject || "Email"}



          </Typography>



          <IconButton size="small" onClick={() => setSelectedEmailId(null)}>



            <CloseRoundedIcon />



          </IconButton>



        </Box>







        {/* Reader Body */}



        <Box sx={{ flex: 1, overflowY: "auto", p: 2.5 }}>



          {loadingDetail ? (



            <Box sx={{ display: "flex", justifyContent: "center", py: 8 }}>



              <CircularProgress size={28} sx={{ color: "#DC4C3E" }} />



            </Box>



          ) : emailDetail ? (



            <Box>



              <Box sx={{ mb: 2, pb: 1.5, borderBottom: "1px solid #F4F4F4" }}>



                <Typography variant="body2" sx={{ fontWeight: 600, color: "#202020" }}>



                  From: <span style={{ fontWeight: 400, color: "#555555" }}>{emailDetail.from}</span>



                </Typography>



                {emailDetail.to.length > 0 && (



                  <Typography variant="caption" sx={{ color: "#777777", display: "block", mt: 0.25 }}>



                    To: {emailDetail.to.join(", ")}



                  </Typography>



                )}



                <Typography variant="caption" sx={{ color: "#999999", display: "block", mt: 0.25 }}>



                  Date: {emailDetail.date ? new Date(emailDetail.date).toLocaleString() : "Unknown"}



                </Typography>



              </Box>







              <Typography



                variant="body2"



                component="pre"



                sx={{



                  whiteSpace: "pre-wrap",



                  wordBreak: "break-word",



                  fontFamily: "inherit",



                  fontSize: "0.85rem",



                  lineHeight: 1.6,



                  color: "#303030",



                }}



              >



                {emailDetail.body_text || "(Empty body content)"}



              </Typography>



            </Box>



          ) : (



            <Typography variant="body2" sx={{ color: "#808080" }}>



              Could not load email body.



            </Typography>



          )}



        </Box>



      </Drawer>



    </Box>



  );



}



